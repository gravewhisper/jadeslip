use crate::search::MAX_QUERY_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const MAX_SAVED_SEARCHES: usize = 200;
pub const MAX_RECENT_SEARCHES: usize = 50;
pub const MAX_SEARCH_NAME_CHARS: usize = 80;
pub(crate) const MAX_SEARCHES_BYTES: usize = 16 << 20;
const VERSION: u64 = 1;

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    pub regex: bool,
    pub match_case: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearch {
    pub name: String,
    #[serde(flatten)]
    pub query: SearchQuery,
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct SearchMemory {
    pub saved: Vec<SavedSearch>,
    pub recent: Vec<SearchQuery>,
}

fn query_problem(query: &SearchQuery) -> Option<String> {
    if query.text.is_empty() {
        return Some("A saved search needs some text to search for.".to_owned());
    }
    (query.text.len() > MAX_QUERY_BYTES).then(|| {
        format!(
            "That search is longer than {} KB, so it cannot be kept.",
            MAX_QUERY_BYTES >> 10
        )
    })
}

fn name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("Give the saved search a name.".to_owned());
    }
    if name.trim() != name || name.chars().any(char::is_control) {
        return Some(
            "A saved search name cannot start or end with spaces or hold line breaks.".to_owned(),
        );
    }
    (name.chars().count() > MAX_SEARCH_NAME_CHARS)
        .then(|| format!("Keep the name to {MAX_SEARCH_NAME_CHARS} characters."))
}

fn name_key(name: &str) -> String {
    name.to_lowercase()
}

impl SearchMemory {
    pub fn parse(bytes: &[u8]) -> SearchMemory {
        let Ok(root) = serde_json::from_slice::<Value>(bytes) else {
            return SearchMemory::default();
        };
        if root["version"].as_u64() != Some(VERSION) {
            return SearchMemory::default();
        }
        let entries = |key: &str| root[key].as_array().cloned().unwrap_or_default();
        let mut names = HashSet::new();
        let mut queries = HashSet::new();
        let saved = entries("saved")
            .into_iter()
            .filter_map(|entry| serde_json::from_value::<SavedSearch>(entry).ok())
            .filter(|entry| {
                name_problem(&entry.name).is_none() && query_problem(&entry.query).is_none()
            })
            .filter(|entry| {
                names.insert(name_key(&entry.name)) && queries.insert(entry.query.clone())
            })
            .take(MAX_SAVED_SEARCHES)
            .collect();
        let mut seen = HashSet::new();
        let recent = entries("recent")
            .into_iter()
            .filter_map(|entry| serde_json::from_value::<SearchQuery>(entry).ok())
            .filter(|query| query_problem(query).is_none() && seen.insert(query.clone()))
            .take(MAX_RECENT_SEARCHES)
            .collect();
        SearchMemory { saved, recent }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.saved.len() > MAX_SAVED_SEARCHES {
            return Err(format!(
                "You have {MAX_SAVED_SEARCHES} saved searches. Delete one to save another."
            ));
        }
        if self.recent.len() > MAX_RECENT_SEARCHES {
            return Err(format!(
                "Recent searches keep at most {MAX_RECENT_SEARCHES}."
            ));
        }
        let mut names = HashSet::new();
        let mut queries = HashSet::new();
        for entry in &self.saved {
            if let Some(problem) = name_problem(&entry.name).or_else(|| query_problem(&entry.query))
            {
                return Err(problem);
            }
            if !names.insert(name_key(&entry.name)) {
                return Err(format!(
                    "A saved search is already called \"{}\".",
                    entry.name
                ));
            }
            if !queries.insert(&entry.query) {
                return Err("That search is already saved under another name.".to_owned());
            }
        }
        let mut seen = HashSet::new();
        for query in &self.recent {
            if let Some(problem) = query_problem(query) {
                return Err(problem);
            }
            if !seen.insert(query) {
                return Err("A recent search is listed twice.".to_owned());
            }
        }
        Ok(())
    }

    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.check()?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": VERSION,
            "saved": self.saved,
            "recent": self.recent,
        }))
        .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_SEARCHES_BYTES {
            return Err(
                "Saved searches are too large to keep. Delete a long one first.".to_owned(),
            );
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str, regex: bool, match_case: bool) -> SearchQuery {
        SearchQuery {
            text: text.to_owned(),
            regex,
            match_case,
        }
    }

    fn saved(name: &str, text: &str) -> SavedSearch {
        SavedSearch {
            name: name.to_owned(),
            query: query(text, true, false),
        }
    }

    #[test]
    fn a_memory_reads_back_in_order() {
        let memory = SearchMemory {
            saved: vec![saved("Romance", r"\bkiss(?:ed)?\b"), saved("Names", "Han Mo")],
            recent: vec![query("blush", false, true), query("(?i)sigh", true, false)],
        };
        assert_eq!(SearchMemory::parse(&memory.to_bytes().unwrap()), memory);
    }

    #[test]
    fn the_file_shape_is_versioned_camel_case() {
        let memory = SearchMemory {
            saved: vec![saved("Romance", "kiss")],
            recent: vec![query("blush", false, true)],
        };
        let value: Value = serde_json::from_slice(&memory.to_bytes().unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "version": 1,
                "saved": [{"name": "Romance", "text": "kiss", "regex": true, "matchCase": false}],
                "recent": [{"text": "blush", "regex": false, "matchCase": true}],
            })
        );
    }

    #[test]
    fn damaged_files_read_as_empty() {
        for bytes in [
            b"".as_slice(),
            b"\xff\xfe".as_slice(),
            b"[1,2]".as_slice(),
            br#"{"version":2,"saved":[{"name":"a","text":"b","regex":false,"matchCase":false}]}"#
                .as_slice(),
            br#"{"saved":[{"name":"a","text":"b","regex":false,"matchCase":false}]}"#.as_slice(),
        ] {
            assert_eq!(SearchMemory::parse(bytes), SearchMemory::default());
        }
    }

    #[test]
    fn bad_entries_are_dropped_and_the_good_ones_kept() {
        let long = "x".repeat(MAX_QUERY_BYTES + 1);
        let long_name = "n".repeat(MAX_SEARCH_NAME_CHARS + 1);
        let file = serde_json::json!({
            "version": 1,
            "saved": [
                {"name": "Good", "text": "kiss", "regex": true, "matchCase": false},
                {"name": "good", "text": "other", "regex": true, "matchCase": false},
                {"name": "Twin", "text": "kiss", "regex": true, "matchCase": false},
                {"name": "", "text": "x", "regex": false, "matchCase": false},
                {"name": " padded", "text": "x", "regex": false, "matchCase": false},
                {"name": "line\nbreak", "text": "x", "regex": false, "matchCase": false},
                {"name": long_name, "text": "x", "regex": false, "matchCase": false},
                {"name": "Long", "text": long, "regex": false, "matchCase": false},
                {"name": "Empty", "text": "", "regex": false, "matchCase": false},
                {"name": "Typed", "text": 7, "regex": false, "matchCase": false},
                {"name": "Kept", "text": "<img src=x onerror=alert(1)>", "regex": false, "matchCase": true},
                "junk"
            ],
            "recent": [
                {"text": "a", "regex": false, "matchCase": false},
                {"text": "a", "regex": false, "matchCase": false},
                {"text": "a", "regex": true, "matchCase": false},
                {"text": "", "regex": false, "matchCase": false},
                {"text": "b", "regex": "yes", "matchCase": false}
            ]
        });
        let memory = SearchMemory::parse(&serde_json::to_vec(&file).unwrap());
        let names: Vec<&str> = memory
            .saved
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["Good", "Kept"]);
        assert_eq!(
            memory.recent,
            [query("a", false, false), query("a", true, false)]
        );
    }

    #[test]
    fn a_file_over_the_limits_keeps_only_the_first_entries() {
        let saved: Vec<Value> = (0..MAX_SAVED_SEARCHES + 30)
            .map(|at| serde_json::json!({"name": format!("s{at}"), "text": format!("t{at}"), "regex": false, "matchCase": false}))
            .collect();
        let recent: Vec<Value> = (0..MAX_RECENT_SEARCHES + 30)
            .map(|at| serde_json::json!({"text": format!("r{at}"), "regex": false, "matchCase": false}))
            .collect();
        let file = serde_json::json!({"version": 1, "saved": saved, "recent": recent});
        let memory = SearchMemory::parse(&serde_json::to_vec(&file).unwrap());
        assert_eq!(memory.saved.len(), MAX_SAVED_SEARCHES);
        assert_eq!(memory.recent.len(), MAX_RECENT_SEARCHES);
        assert_eq!(memory.recent[0].text, "r0");
    }

    #[test]
    fn writing_refuses_what_reading_would_drop() {
        let refused = [
            SearchMemory {
                saved: vec![saved("A", "x"), saved("a", "y")],
                ..SearchMemory::default()
            },
            SearchMemory {
                saved: vec![saved("A", "x"), saved("B", "x")],
                ..SearchMemory::default()
            },
            SearchMemory {
                saved: vec![saved("", "x")],
                ..SearchMemory::default()
            },
            SearchMemory {
                saved: vec![saved("A", &"x".repeat(MAX_QUERY_BYTES + 1))],
                ..SearchMemory::default()
            },
            SearchMemory {
                saved: (0..=MAX_SAVED_SEARCHES)
                    .map(|at| saved(&format!("s{at}"), &format!("t{at}")))
                    .collect(),
                ..SearchMemory::default()
            },
            SearchMemory {
                recent: (0..=MAX_RECENT_SEARCHES)
                    .map(|at| query(&format!("r{at}"), false, false))
                    .collect(),
                ..SearchMemory::default()
            },
            SearchMemory {
                recent: vec![query("a", false, false), query("a", false, false)],
                ..SearchMemory::default()
            },
        ];
        for memory in refused {
            assert!(memory.to_bytes().is_err(), "{memory:?}");
        }
    }

    #[test]
    fn a_full_memory_of_long_patterns_still_fits_the_file_cap() {
        let pattern = "\\".repeat(MAX_QUERY_BYTES / 2);
        let memory = SearchMemory {
            saved: (0..MAX_SAVED_SEARCHES)
                .map(|at| saved(&format!("s{at}"), &format!("{at}{pattern}")))
                .collect(),
            recent: Vec::new(),
        };
        let bytes = memory.to_bytes().unwrap();
        assert!(bytes.len() <= MAX_SEARCHES_BYTES);
        assert_eq!(SearchMemory::parse(&bytes), memory);
    }
}
