use crate::Error;
use encoding_rs::{Encoding, UTF_8};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const DICTIONARIES_DIR: &str = "dictionaries";
const MAX_DICTIONARY_BYTES: u64 = 64 << 20;
const NEUTRAL: [&str; 3] = ["und", "mul", "zxx"];
const SAME_LANGUAGE: &[&[&str]] = &[&["tl", "fil", "tgl"]];

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "code")]
pub enum SpellChoice {
    #[default]
    Auto,
    English,
    Installed(String),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "code")]
pub enum SpellLanguage {
    English,
    Installed(String),
}

fn normalized(tag: &str) -> String {
    tag.trim().replace('_', "-").to_ascii_lowercase()
}

fn family(tag: &str) -> String {
    let tag = normalized(tag);
    let primary = tag.split('-').next().unwrap_or("");
    SAME_LANGUAGE
        .iter()
        .find(|group| group.contains(&primary))
        .map_or(primary, |group| group[0])
        .to_owned()
}

fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 32
        && code
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

pub(super) fn english_applies(language: Option<&str>) -> bool {
    let Some(language) = language else {
        return true;
    };
    let primary = family(language);
    primary.is_empty() || primary == "en" || NEUTRAL.contains(&primary.as_str())
}

pub fn dictionaries_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DICTIONARIES_DIR)
}

pub fn installed(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut codes: Vec<String> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "aff" {
                return None;
            }
            let code = path.file_stem()?.to_str()?.to_owned();
            (is_code(&code) && path.is_file() && path.with_extension("dic").is_file())
                .then_some(code)
        })
        .collect();
    codes.sort();
    codes
}

pub fn resolve(
    choice: &SpellChoice,
    book_language: Option<&str>,
    installed: &[String],
) -> Option<SpellLanguage> {
    match choice {
        SpellChoice::English => return Some(SpellLanguage::English),
        SpellChoice::Installed(code) if installed.contains(code) => {
            return Some(SpellLanguage::Installed(code.clone()));
        }
        SpellChoice::Installed(_) | SpellChoice::Auto => {}
    }
    if let Some(tag) = book_language.filter(|tag| !tag.trim().is_empty()) {
        let wanted = normalized(tag);
        let exact = installed.iter().find(|code| normalized(code) == wanted);
        let primary = family(tag);
        let near = || installed.iter().find(|code| family(code) == primary);
        if let Some(code) = exact.or_else(near) {
            return Some(SpellLanguage::Installed(code.clone()));
        }
    }
    english_applies(book_language).then_some(SpellLanguage::English)
}

pub fn passage_tags(language: &SpellLanguage) -> Vec<String> {
    let primary = match language {
        SpellLanguage::English => "en".to_owned(),
        SpellLanguage::Installed(code) => family(code),
    };
    let mut tags: Vec<String> = SAME_LANGUAGE
        .iter()
        .find(|group| group[0] == primary)
        .map_or_else(
            || vec![primary.clone()],
            |group| group.iter().map(|tag| (*tag).to_owned()).collect(),
        );
    tags.extend(NEUTRAL.iter().map(|tag| (*tag).to_owned()));
    tags
}

fn read_capped(path: &Path) -> Result<Vec<u8>, Error> {
    let unreadable = |why: &str| {
        Error::Book(format!(
            "The dictionary file {} could not be read. {why}",
            path.display()
        ))
    };
    let file = fs::File::open(path).map_err(|error| unreadable(crate::plain_io(&error)))?;
    let mut bytes = Vec::new();
    file.take(MAX_DICTIONARY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(crate::plain_io(&error)))?;
    if bytes.len() as u64 > MAX_DICTIONARY_BYTES {
        return Err(unreadable("It is larger than 64 MB."));
    }
    Ok(bytes)
}

fn declared_encoding(aff: &[u8]) -> Result<&'static Encoding, Error> {
    let label = aff.split(|byte| *byte == b'\n').find_map(|line| {
        let line = line.trim_ascii();
        let rest = line.strip_prefix(b"SET")?;
        rest.first()
            .is_some_and(u8::is_ascii_whitespace)
            .then(|| rest.trim_ascii())
    });
    let Some(label) = label else {
        return Ok(UTF_8);
    };
    let label = label.strip_prefix(b"microsoft-").unwrap_or(label);
    Encoding::for_label(label).ok_or_else(|| {
        Error::Book(format!(
            "The dictionary names a character set this app does not know: {}.",
            String::from_utf8_lossy(label)
        ))
    })
}

fn decoded(bytes: &[u8], encoding: &'static Encoding, path: &Path) -> Result<String, Error> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|text| text.replace('\r', ""))
        .ok_or_else(|| {
            Error::Book(format!(
                "The dictionary file {} is not valid {}.",
                path.display(),
                encoding.name()
            ))
        })
}

pub(super) fn read_pair(aff: &Path, dic: &Path) -> Result<(String, String), Error> {
    let aff_bytes = read_capped(aff)?;
    let dic_bytes = read_capped(dic)?;
    let encoding = declared_encoding(&aff_bytes)?;
    Ok((
        decoded(&aff_bytes, encoding, aff)?,
        decoded(&dic_bytes, encoding, dic)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|code| (*code).to_owned()).collect()
    }

    fn installed_one(code: &str) -> SpellLanguage {
        SpellLanguage::Installed(code.to_owned())
    }

    #[test]
    fn auto_follows_the_book_tag_by_its_primary_subtag() {
        let have = codes(&["de_DE", "fr", "tl"]);
        for (tag, expected) in [
            (Some("fr-FR"), Some(installed_one("fr"))),
            (Some("FR"), Some(installed_one("fr"))),
            (Some("de"), Some(installed_one("de_DE"))),
            (Some("de-DE"), Some(installed_one("de_DE"))),
            (Some("tl"), Some(installed_one("tl"))),
            (Some("fil"), Some(installed_one("tl"))),
            (Some("fil-PH"), Some(installed_one("tl"))),
            (Some("en-GB"), Some(SpellLanguage::English)),
            (None, Some(SpellLanguage::English)),
            (Some(""), Some(SpellLanguage::English)),
            (Some("und"), Some(SpellLanguage::English)),
            (Some("es"), None),
            (Some("eng"), None),
        ] {
            assert_eq!(resolve(&SpellChoice::Auto, tag, &have), expected, "{tag:?}");
        }
        assert_eq!(
            resolve(&SpellChoice::Auto, Some("tl"), &codes(&["fil_PH"])),
            Some(installed_one("fil_PH"))
        );
    }

    #[test]
    fn an_exact_tag_wins_over_a_primary_match() {
        let have = codes(&["en_AU", "en_GB"]);
        assert_eq!(
            resolve(&SpellChoice::Auto, Some("en-GB"), &have),
            Some(installed_one("en_GB"))
        );
    }

    #[test]
    fn a_manual_choice_wins_and_a_missing_one_falls_back_to_auto() {
        let have = codes(&["fr"]);
        assert_eq!(
            resolve(&SpellChoice::English, Some("fr"), &have),
            Some(SpellLanguage::English)
        );
        assert_eq!(
            resolve(&SpellChoice::Installed("fr".to_owned()), Some("en"), &have),
            Some(installed_one("fr"))
        );
        assert_eq!(
            resolve(
                &SpellChoice::Installed("gone".to_owned()),
                Some("fr"),
                &have
            ),
            Some(installed_one("fr"))
        );
        assert_eq!(
            resolve(
                &SpellChoice::Installed("gone".to_owned()),
                Some("es"),
                &have
            ),
            None
        );
    }

    #[test]
    fn passage_tags_name_the_language_its_aliases_and_the_neutral_tags() {
        assert_eq!(
            passage_tags(&SpellLanguage::English),
            codes(&["en", "und", "mul", "zxx"])
        );
        assert_eq!(
            passage_tags(&installed_one("fil_PH")),
            codes(&["tl", "fil", "tgl", "und", "mul", "zxx"])
        );
        assert_eq!(
            passage_tags(&installed_one("de_DE")),
            codes(&["de", "und", "mul", "zxx"])
        );
    }

    #[test]
    fn only_a_complete_safely_named_pair_counts_as_installed() {
        let dir = jadeslip_fixtures::scratch::scratch("spell-installed");
        for name in [
            "fr.aff",
            "fr.dic",
            "de.aff",
            "es.dic",
            "bad name.aff",
            "bad name.dic",
        ] {
            fs::write(dir.join(name), "").unwrap();
        }
        fs::create_dir_all(dir.join("it.aff")).unwrap();
        fs::write(dir.join("it.dic"), "").unwrap();
        assert_eq!(installed(&dir), codes(&["fr"]));
        assert_eq!(installed(&dir.join("missing")), Vec::<String>::new());
    }

    #[test]
    fn the_set_line_picks_the_character_set() {
        for (aff, expected) in [
            (b"SET UTF-8\n".as_slice(), "UTF-8"),
            (
                b"# note\r\nSET ISO8859-1\r\nTRY abc\r\n".as_slice(),
                "windows-1252",
            ),
            (b"SET microsoft-cp1251\n".as_slice(), "windows-1251"),
            (b"TRY abc\n".as_slice(), "UTF-8"),
            (b"SETTLE x\n".as_slice(), "UTF-8"),
        ] {
            assert_eq!(declared_encoding(aff).unwrap().name(), expected);
        }
        assert!(declared_encoding(b"SET KLINGON-9\n").is_err());
    }
}
