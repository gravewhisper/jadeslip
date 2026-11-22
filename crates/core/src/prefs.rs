use crate::Error;
use crate::searches::{MAX_SEARCHES_BYTES, SearchMemory};
use crate::temp::TempFile;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PREFS_FILE: &str = "prefs.json";
const SEARCHES_FILE: &str = "searches.json";
const MAX_FILE_BYTES: u64 = 4 << 20;
pub const MIN_FONT_PX: u16 = 14;
pub const MAX_FONT_PX: u16 = 28;
const DEFAULT_FONT_PX: u16 = 18;
const DEFAULT_LINE_HEIGHT: u16 = 160;
const DEFAULT_PARA_GAP: u16 = 60;
const DEFAULT_WIDTH_PX: u16 = 636;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadingFont {
    #[default]
    Literata,
    PlexSans,
    Serif,
    Sans,
    Mono,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pages {
    #[default]
    Scroll,
    #[serde(alias = "one", alias = "two")]
    Auto,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShelfView {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShelfSort {
    #[default]
    Opened,
    Title,
    Author,
    Progress,
    Chapters,
    Size,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Keep {
    Days7,
    Days30,
    Days90,
    Year,
    #[default]
    Forever,
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

impl Keep {
    pub fn cutoff_ms(self, now_ms: u64) -> Option<u64> {
        let days = match self {
            Keep::Days7 => 7,
            Keep::Days30 => 30,
            Keep::Days90 => 90,
            Keep::Year => 365,
            Keep::Forever => return None,
        };
        Some(now_ms.saturating_sub(days * DAY_MS))
    }
}

fn lenient<'de, D, T>(from: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(from)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn clamp_number(value: &serde_json::Value, min: u16, max: u16, default: u16) -> u16 {
    value.as_u64().map_or(default, |number| {
        number.clamp(u64::from(min), u64::from(max)) as u16
    })
}

fn clamped<'de, D, const MIN: u16, const MAX: u16, const DEFAULT: u16>(
    from: D,
) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(from)?;
    Ok(clamp_number(&value, MIN, MAX, DEFAULT))
}

const fn default_of<const VALUE: u16>() -> u16 {
    VALUE
}

fn width_px<'de, D: Deserializer<'de>>(from: D) -> Result<u16, D::Error> {
    let value = serde_json::Value::deserialize(from)?;
    Ok(match value.as_str() {
        Some("narrow") => 516,
        Some("medium") => DEFAULT_WIDTH_PX,
        Some("wide") => 816,
        _ => clamp_number(&value, 400, 1400, DEFAULT_WIDTH_PX),
    })
}

const fn default_shown() -> bool {
    true
}

fn shown<'de, D: Deserializer<'de>>(from: D) -> Result<bool, D::Error> {
    let value = serde_json::Value::deserialize(from)?;
    Ok(value.as_bool().unwrap_or(true))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    #[serde(default, deserialize_with = "lenient")]
    pub theme: Theme,
    #[serde(
        default = "default_of::<DEFAULT_FONT_PX>",
        deserialize_with = "clamped::<_, MIN_FONT_PX, MAX_FONT_PX, DEFAULT_FONT_PX>"
    )]
    pub font_px: u16,
    #[serde(
        default = "default_of::<DEFAULT_WIDTH_PX>",
        deserialize_with = "width_px"
    )]
    pub width: u16,
    #[serde(default = "default_shown", deserialize_with = "shown")]
    pub contents: bool,
    #[serde(
        default = "default_of::<44>",
        deserialize_with = "clamped::<_, 0, 200, 44>"
    )]
    pub margin_top: u16,
    #[serde(
        default = "default_of::<72>",
        deserialize_with = "clamped::<_, 0, 200, 72>"
    )]
    pub margin_bottom: u16,
    #[serde(
        default = "default_of::<32>",
        deserialize_with = "clamped::<_, 0, 160, 32>"
    )]
    pub margin_left: u16,
    #[serde(
        default = "default_of::<32>",
        deserialize_with = "clamped::<_, 0, 160, 32>"
    )]
    pub margin_right: u16,
    #[serde(
        default = "default_of::<DEFAULT_LINE_HEIGHT>",
        deserialize_with = "clamped::<_, 120, 240, DEFAULT_LINE_HEIGHT>"
    )]
    pub line_height: u16,
    #[serde(
        default = "default_of::<DEFAULT_PARA_GAP>",
        deserialize_with = "clamped::<_, 0, 250, DEFAULT_PARA_GAP>"
    )]
    pub para_gap: u16,
    #[serde(
        default = "default_of::<0>",
        deserialize_with = "clamped::<_, 0, 30, 0>"
    )]
    pub indent: u16,
    #[serde(default, deserialize_with = "lenient")]
    pub font: ReadingFont,
    #[serde(default = "default_shown", deserialize_with = "shown")]
    pub spelling: bool,
    #[serde(default, deserialize_with = "lenient")]
    pub pages: Pages,
    #[serde(default, deserialize_with = "lenient")]
    pub match_case: bool,
    #[serde(default, deserialize_with = "lenient")]
    pub shelf_view: ShelfView,
    #[serde(default, deserialize_with = "lenient")]
    pub shelf_sort: ShelfSort,
    #[serde(default = "default_shown", deserialize_with = "shown")]
    pub shelf_recent: bool,
    #[serde(default, deserialize_with = "lenient")]
    pub keep_snapshots: Keep,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            theme: Theme::default(),
            font_px: DEFAULT_FONT_PX,
            width: DEFAULT_WIDTH_PX,
            contents: true,
            margin_top: 44,
            margin_bottom: 72,
            margin_left: 32,
            margin_right: 32,
            line_height: DEFAULT_LINE_HEIGHT,
            para_gap: DEFAULT_PARA_GAP,
            indent: 0,
            font: ReadingFont::Literata,
            spelling: true,
            pages: Pages::Scroll,
            match_case: false,
            shelf_view: ShelfView::Grid,
            shelf_sort: ShelfSort::Opened,
            shelf_recent: true,
            keep_snapshots: Keep::Forever,
        }
    }
}

pub struct Store {
    dir: PathBuf,
}

fn read_small(path: &Path, most: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(most + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= most).then_some(bytes)
}

impl Store {
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    pub fn prefs(&self) -> Prefs {
        let path = self.dir.join(PREFS_FILE);
        if !path.exists() {
            return Prefs::default();
        }
        read_small(&path, MAX_FILE_BYTES)
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .filter(serde_json::Value::is_object)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or(Prefs {
                keep_snapshots: Keep::Forever,
                ..Prefs::default()
            })
    }

    pub fn set_prefs(&self, prefs: &Prefs) -> Result<(), Error> {
        self.write(PREFS_FILE, prefs)
    }

    pub fn searches(&self) -> SearchMemory {
        read_small(&self.dir.join(SEARCHES_FILE), MAX_SEARCHES_BYTES as u64)
            .map(|bytes| SearchMemory::parse(&bytes))
            .unwrap_or_default()
    }

    pub fn set_searches(&self, memory: &SearchMemory) -> Result<(), Error> {
        self.write_bytes(SEARCHES_FILE, memory.to_bytes().map_err(Error::Book)?)
    }

    fn write(&self, name: &str, value: &impl Serialize) -> Result<(), Error> {
        let bytes = serde_json::to_vec(value).map_err(|error| Error::Book(error.to_string()))?;
        self.write_bytes(name, bytes)
    }

    fn write_bytes(&self, name: &str, bytes: Vec<u8>) -> Result<(), Error> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(name);
        let temp = TempFile::beside(&path)?;
        temp.file().write_all(&bytes)?;
        temp.replace(&path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        jadeslip_fixtures::scratch::scratch(&format!("prefs-{name}"))
    }

    #[test]
    fn prefs_survive_a_new_store() {
        let dir = scratch("survive");
        let chosen = Prefs {
            theme: Theme::Light,
            font_px: 23,
            width: 1012,
            contents: false,
            margin_top: 0,
            margin_bottom: 120,
            margin_left: 8,
            margin_right: 96,
            line_height: 210,
            para_gap: 40,
            indent: 15,
            font: ReadingFont::Mono,
            spelling: false,
            pages: Pages::Auto,
            match_case: true,
            shelf_view: ShelfView::List,
            shelf_sort: ShelfSort::Chapters,
            shelf_recent: false,
            keep_snapshots: Keep::Days30,
        };
        Store::new(dir.clone()).set_prefs(&chosen).unwrap();
        assert_eq!(Store::new(dir).prefs(), chosen);
    }

    #[test]
    fn searches_survive_a_new_store_and_a_missing_file_is_empty() {
        let dir = scratch("searches");
        let store = Store::new(dir.clone());
        assert_eq!(store.searches(), SearchMemory::default());
        let memory = SearchMemory {
            saved: vec![crate::SavedSearch {
                name: "Romance".to_owned(),
                query: crate::SearchQuery {
                    text: r"\bkiss\b".to_owned(),
                    regex: true,
                    match_case: true,
                },
            }],
            recent: Vec::new(),
        };
        store.set_searches(&memory).unwrap();
        assert_eq!(Store::new(dir.clone()).searches(), memory);
        assert_eq!(Store::new(dir).prefs(), Prefs::default());
    }

    #[test]
    fn a_missing_folder_gives_defaults_and_is_made_on_write() {
        let dir = scratch("missing").join("deeper/still");
        let store = Store::new(dir);
        assert_eq!(store.prefs(), Prefs::default());
        store.set_prefs(&Prefs::default()).unwrap();
        assert_eq!(store.prefs(), Prefs::default());
    }

    #[test]
    fn corrupt_files_fall_back_to_defaults_and_keep_every_point() {
        let kept = Prefs {
            keep_snapshots: Keep::Forever,
            ..Prefs::default()
        };
        for (name, bytes) in [
            ("garbage", b"\xff\xfe not json at all".as_slice()),
            ("empty", b"".as_slice()),
            ("wrong shape", b"[1,2,3]".as_slice()),
            ("truncated", b"{\"theme\":\"li".as_slice()),
        ] {
            let dir = scratch(&format!("corrupt-{}", name.replace(' ', "-")));
            std::fs::write(dir.join(PREFS_FILE), bytes).unwrap();
            let store = Store::new(dir);
            assert_eq!(store.prefs(), kept, "{name}");
        }
    }

    #[test]
    fn one_bad_field_keeps_the_good_ones() {
        let dir = scratch("field");
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"theme":"light","fontPx":"huge","width":"enormous","contents":"maybe","spelling":0,"later":1}"#,
        )
        .unwrap();
        let prefs = Store::new(dir).prefs();
        assert_eq!(prefs.theme, Theme::Light);
        assert_eq!(prefs.font_px, DEFAULT_FONT_PX);
        assert_eq!(prefs.width, DEFAULT_WIDTH_PX);
        assert!(prefs.contents);
        assert!(prefs.spelling);
    }

    #[test]
    fn the_page_mode_reads_back_old_page_counts_become_auto_and_an_unknown_one_scrolls() {
        for (at, (text, pages)) in [
            ("\"auto\"", Pages::Auto),
            ("\"one\"", Pages::Auto),
            ("\"two\"", Pages::Auto),
            ("\"scroll\"", Pages::Scroll),
            ("\"three\"", Pages::Scroll),
            ("2", Pages::Scroll),
        ]
        .into_iter()
        .enumerate()
        {
            let dir = scratch(&format!("pages-{at}"));
            std::fs::write(
                dir.join(PREFS_FILE),
                format!("{{\"pages\":{text},\"theme\":\"light\"}}"),
            )
            .unwrap();
            let prefs = Store::new(dir).prefs();
            assert_eq!(prefs.pages, pages, "{text}");
            assert_eq!(prefs.theme, Theme::Light, "{text}");
        }
    }

    #[test]
    fn the_old_named_widths_become_the_same_text_column() {
        for (at, (old, column)) in [
            ("\"narrow\"", 516),
            ("\"medium\"", 636),
            ("\"wide\"", 816),
            ("\"; display:none\"", 636),
            ("7", 400),
            ("99999", 1400),
            ("null", 636),
        ]
        .into_iter()
        .enumerate()
        {
            let dir = scratch(&format!("width-{at}"));
            std::fs::write(dir.join(PREFS_FILE), format!("{{\"width\":{old}}}")).unwrap();
            assert_eq!(Store::new(dir).prefs().width, column, "{old}");
        }
    }

    #[test]
    fn saved_reading_values_win_over_new_defaults() {
        let dir = scratch("saved-reading");
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"fontPx":19,"lineHeight":172,"paraGap":105,"pages":"auto"}"#,
        )
        .unwrap();
        let prefs = Store::new(dir.clone()).prefs();
        assert_eq!((prefs.font_px, prefs.line_height, prefs.para_gap), (19, 172, 105));
        assert_eq!(prefs.pages, Pages::Auto);
        std::fs::write(dir.join(PREFS_FILE), br#"{"theme":"light"}"#).unwrap();
        let fresh = Store::new(dir).prefs();
        assert_eq!((fresh.font_px, fresh.line_height, fresh.para_gap), (18, 160, 60));
    }

    #[test]
    fn hostile_layout_values_fall_back_or_clamp() {
        let dir = scratch("layout-hostile");
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"marginTop":-40,"marginBottom":"1em; color:red","marginLeft":1e300,"marginRight":12.6,"lineHeight":"2","paraGap":9999,"width":1e30,"indent":-1,"font":"Comic Sans; }"}"#,
        )
        .unwrap();
        let prefs = Store::new(dir).prefs();
        assert_eq!(prefs.margin_top, 44);
        assert_eq!(prefs.margin_bottom, 72);
        assert_eq!(prefs.margin_left, 32);
        assert_eq!(prefs.margin_right, 32);
        assert_eq!(prefs.line_height, DEFAULT_LINE_HEIGHT);
        assert_eq!(prefs.para_gap, 250);
        assert_eq!(prefs.width, DEFAULT_WIDTH_PX);
        assert_eq!(prefs.indent, 0);
        assert_eq!(prefs.font, ReadingFont::Literata);
    }

    #[test]
    fn a_file_from_before_the_layout_controls_keeps_todays_look() {
        let dir = scratch("layout-old");
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"theme":"light","fontPx":22,"width":"medium","contents":true}"#,
        )
        .unwrap();
        let prefs = Store::new(dir).prefs();
        assert_eq!(
            prefs,
            Prefs {
                theme: Theme::Light,
                font_px: 22,
                keep_snapshots: Keep::Forever,
                ..Prefs::default()
            }
        );
    }

    #[test]
    fn a_file_from_before_match_case_searches_ignoring_case() {
        let dir = scratch("match-case-old");
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"theme":"light","pages":"one","spelling":false}"#,
        )
        .unwrap();
        let prefs = Store::new(dir).prefs();
        assert!(!prefs.match_case);
        assert_eq!(prefs.pages, Pages::Auto);
        assert!(!Prefs::default().match_case);
    }

    #[test]
    fn match_case_reads_back_and_a_strange_value_turns_it_off() {
        for (at, (text, on)) in [
            ("true", true),
            ("false", false),
            ("\"yes\"", false),
            ("1", false),
            ("null", false),
        ]
        .into_iter()
        .enumerate()
        {
            let dir = scratch(&format!("match-case-{at}"));
            std::fs::write(
                dir.join(PREFS_FILE),
                format!("{{\"matchCase\":{text},\"theme\":\"light\"}}"),
            )
            .unwrap();
            let prefs = Store::new(dir).prefs();
            assert_eq!(prefs.match_case, on, "{text}");
            assert_eq!(prefs.theme, Theme::Light, "{text}");
        }
    }

    #[test]
    fn the_shelf_view_and_sort_read_back_and_strange_ones_fall_back() {
        let dir = scratch("shelf");
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"theme":"light","shelfView":"list","shelfSort":"author","shelfRecent":false}"#,
        )
        .unwrap();
        let prefs = Store::new(dir.clone()).prefs();
        assert_eq!(
            (prefs.shelf_view, prefs.shelf_sort, prefs.shelf_recent),
            (ShelfView::List, ShelfSort::Author, false)
        );
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"theme":"light","shelfView":"<b>","shelfSort":7,"shelfRecent":"no"}"#,
        )
        .unwrap();
        let prefs = Store::new(dir).prefs();
        assert_eq!(prefs.theme, Theme::Light);
        assert_eq!(
            (prefs.shelf_view, prefs.shelf_sort, prefs.shelf_recent),
            (ShelfView::Grid, ShelfSort::Opened, true)
        );
    }

    #[test]
    fn a_font_size_outside_the_range_is_clamped() {
        for (text, expected) in [("3", MIN_FONT_PX), ("400000", MAX_FONT_PX), ("20", 20)] {
            let dir = scratch(&format!("font-{text}"));
            std::fs::write(dir.join(PREFS_FILE), format!("{{\"fontPx\":{text}}}")).unwrap();
            assert_eq!(Store::new(dir).prefs().font_px, expected);
        }
    }

    #[test]
    fn an_oversize_file_is_refused_and_a_directory_in_the_way_does_not_panic() {
        let dir = scratch("oversize");
        std::fs::write(
            dir.join(PREFS_FILE),
            vec![b' '; MAX_FILE_BYTES as usize + 8],
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("prefs.json.tmp")).unwrap();
        let store = Store::new(dir);
        assert_eq!(
            store.prefs(),
            Prefs {
                keep_snapshots: Keep::Forever,
                ..Prefs::default()
            }
        );
    }

    #[test]
    fn a_file_from_before_snapshot_cleanup_keeps_every_point_and_its_other_values() {
        let dir = scratch("keep-old");
        std::fs::write(
            dir.join(PREFS_FILE),
            br#"{"theme":"light","fontPx":21,"pages":"auto","shelfView":"list"}"#,
        )
        .unwrap();
        let prefs = Store::new(dir).prefs();
        assert_eq!(prefs.keep_snapshots, Keep::Forever);
        assert_eq!(
            (prefs.theme, prefs.font_px, prefs.pages, prefs.shelf_view),
            (Theme::Light, 21, Pages::Auto, ShelfView::List)
        );
    }

    #[test]
    fn a_fresh_store_keeps_every_point() {
        let store = Store::new(scratch("keep-fresh"));
        assert_eq!(store.prefs().keep_snapshots, Keep::Forever);
    }

    #[test]
    fn a_strange_keep_value_keeps_every_point() {
        for (at, text) in ["\"days3\"", "7", "null", "\"Days90\"", "{}"]
            .into_iter()
            .enumerate()
        {
            let dir = scratch(&format!("keep-bad-{at}"));
            std::fs::write(
                dir.join(PREFS_FILE),
                format!("{{\"keepSnapshots\":{text},\"theme\":\"light\"}}"),
            )
            .unwrap();
            let prefs = Store::new(dir).prefs();
            assert_eq!(prefs.keep_snapshots, Keep::Forever, "{text}");
            assert_eq!(prefs.theme, Theme::Light, "{text}");
        }
    }

    #[test]
    fn every_keep_choice_reads_back_under_its_name() {
        for (keep, name) in [
            (Keep::Days7, "days7"),
            (Keep::Days30, "days30"),
            (Keep::Days90, "days90"),
            (Keep::Year, "year"),
            (Keep::Forever, "forever"),
        ] {
            let dir = scratch(&format!("keep-{name}"));
            let chosen = Prefs {
                keep_snapshots: keep,
                ..Prefs::default()
            };
            Store::new(dir.clone()).set_prefs(&chosen).unwrap();
            let text = std::fs::read_to_string(dir.join(PREFS_FILE)).unwrap();
            assert!(
                text.contains(&format!("\"keepSnapshots\":\"{name}\"")),
                "{text}"
            );
            assert_eq!(Store::new(dir).prefs(), chosen);
        }
    }

    #[test]
    fn a_cutoff_is_that_many_days_before_now_and_forever_has_none() {
        let now = 400 * DAY_MS;
        assert_eq!(Keep::Days7.cutoff_ms(now), Some(393 * DAY_MS));
        assert_eq!(Keep::Days30.cutoff_ms(now), Some(370 * DAY_MS));
        assert_eq!(Keep::Days90.cutoff_ms(now), Some(310 * DAY_MS));
        assert_eq!(Keep::Year.cutoff_ms(now), Some(35 * DAY_MS));
        assert_eq!(Keep::Forever.cutoff_ms(now), None);
        assert_eq!(Keep::Year.cutoff_ms(DAY_MS), Some(0));
    }
}
