use crate::{BlockIx, ChapterIx, DomPoint, DomRange, Error, ReadAt, Reader, VisRange};
use spellbook::Dictionary;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub mod dictionaries;

const AFF: &str = include_str!("../dict/en_US.aff");
const DIC: &str = include_str!("../dict/en_US.dic");
const MAX_WORD_BYTES: usize = 40;
const MAX_SUGGESTIONS: usize = 5;
const MAX_USER_LIST_BYTES: u64 = 1 << 20;
pub const MAX_MARKS: usize = 5_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Misspelling {
    pub range: DomRange,
}

pub struct Speller {
    dictionary: RwLock<Dictionary>,
    user_words: Option<PathBuf>,
}

fn is_apostrophe(ch: char) -> bool {
    matches!(ch, '\'' | '\u{2019}' | '\u{2BC}')
}

fn is_joiner(ch: char) -> bool {
    matches!(ch, '\u{AD}' | '\u{200C}' | '\u{200D}' | '\u{2060}')
}

fn is_combining(ch: char) -> bool {
    matches!(ch, '\u{300}'..='\u{36F}')
}

fn in_word(ch: char) -> bool {
    ch.is_alphanumeric() || is_apostrophe(ch) || is_joiner(ch) || is_combining(ch)
}

fn is_latin(ch: char) -> bool {
    matches!(ch, 'a'..='z' | 'A'..='Z' | '\u{C0}'..='\u{24F}' | '\u{1E00}'..='\u{1EFF}')
        && ch != '\u{D7}'
        && ch != '\u{F7}'
}

fn looks_like_address(chunk: &str) -> bool {
    chunk.contains("://") || chunk.contains("www.") || chunk.contains('@')
}

fn trimmed_word(text: &str, span: Range<usize>) -> Option<Range<usize>> {
    let word = text.get(span.clone())?;
    let edge = |ch: char| is_apostrophe(ch) || is_joiner(ch);
    let inner = word.trim_matches(edge);
    if inner.is_empty() {
        return None;
    }
    let start = span.start + (word.len() - word.trim_start_matches(edge).len());
    Some(start..start + inner.len())
}

fn words(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut offset = 0;
    for chunk in text.split_inclusive(char::is_whitespace) {
        if !looks_like_address(chunk) {
            for word in chunk.split(|ch| !in_word(ch)) {
                if !word.is_empty() {
                    let start = offset + (word.as_ptr() as usize - chunk.as_ptr() as usize);
                    found.extend(trimmed_word(text, start..start + word.len()));
                }
            }
        }
        offset += chunk.len();
    }
    found
}

fn is_checkable(word: &str) -> bool {
    if word.len() > MAX_WORD_BYTES {
        return false;
    }
    let mut letters = 0;
    let mut lower = false;
    for ch in word.chars() {
        if is_apostrophe(ch) || is_joiner(ch) || is_combining(ch) {
            continue;
        }
        if !is_latin(ch) {
            return false;
        }
        letters += 1;
        lower |= ch.is_lowercase();
    }
    letters >= 2 && lower
}

fn plain_form(word: &str) -> String {
    word.chars()
        .filter(|ch| !is_joiner(*ch))
        .map(|ch| if is_apostrophe(ch) { '\'' } else { ch })
        .collect()
}

fn accent_free(word: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(word.len());
    for ch in word.chars() {
        let base = match ch {
            '\u{300}'..='\u{36F}' => None,
            '\u{C0}'..='\u{C5}' => Some('A'),
            '\u{C7}' => Some('C'),
            '\u{C8}'..='\u{CB}' => Some('E'),
            '\u{CC}'..='\u{CF}' => Some('I'),
            '\u{D1}' => Some('N'),
            '\u{D2}'..='\u{D6}' | '\u{D8}' => Some('O'),
            '\u{D9}'..='\u{DC}' => Some('U'),
            '\u{DD}' => Some('Y'),
            '\u{E0}'..='\u{E5}' => Some('a'),
            '\u{E7}' => Some('c'),
            '\u{E8}'..='\u{EB}' => Some('e'),
            '\u{EC}'..='\u{EF}' => Some('i'),
            '\u{F1}' => Some('n'),
            '\u{F2}'..='\u{F6}' | '\u{F8}' => Some('o'),
            '\u{F9}'..='\u{FC}' => Some('u'),
            '\u{FD}' | '\u{FF}' => Some('y'),
            _ => {
                out.push(ch);
                continue;
            }
        };
        changed = true;
        out.extend(base);
    }
    changed.then_some(out)
}

fn accepts(dictionary: &Dictionary, word: &str) -> bool {
    let plain = plain_form(word);
    dictionary.check(&plain) || accent_free(&plain).is_some_and(|folded| dictionary.check(&folded))
}

fn name_key(word: &str) -> &str {
    word.split(is_apostrophe).next().unwrap_or(word)
}

fn starts_sentence(text: &str, at: usize) -> bool {
    let before = text[..at].trim_end_matches(|ch: char| {
        matches!(
            ch,
            ' ' | '\t' | '\u{A0}' | '"' | '\'' | '(' | '[' | '\u{2018}' | '\u{201C}' | '\u{2014}'
        )
    });
    before.chars().last().is_none_or(|ch| {
        matches!(
            ch,
            '\n' | '.' | '!' | '?' | ':' | '\u{2026}' | '\u{2019}' | '\u{201D}'
        )
    })
}

fn misspelled_spans(dictionary: &Dictionary, text: &str) -> Vec<Range<usize>> {
    let mut spans = words(text);
    spans.retain(|span| {
        let word = &text[span.clone()];
        is_checkable(word) && !accepts(dictionary, word)
    });
    let names: HashSet<&str> = spans
        .iter()
        .filter(|span| {
            text[span.start..].starts_with(char::is_uppercase) && !starts_sentence(text, span.start)
        })
        .map(|span| name_key(&text[span.clone()]))
        .collect();
    spans.retain(|span| !names.contains(name_key(&text[span.clone()])));
    spans
}

impl Speller {
    pub fn new(user_words: Option<PathBuf>) -> Speller {
        let dictionary =
            Dictionary::new(AFF, DIC).expect("the bundled dictionary is checked by a test");
        Speller::with_words(dictionary, user_words)
    }

    pub fn load(aff: &Path, dic: &Path, user_words: Option<PathBuf>) -> Result<Speller, Error> {
        let (aff_text, dic_text) = dictionaries::read_pair(aff, dic)?;
        let parsed = catch_unwind(AssertUnwindSafe(|| Dictionary::new(&aff_text, &dic_text)));
        let dictionary = match parsed {
            Ok(Ok(dictionary)) => dictionary,
            Ok(Err(error)) => {
                return Err(Error::Book(format!(
                    "The dictionary {} could not be read: {error}.",
                    aff.display()
                )));
            }
            Err(_) => {
                return Err(Error::Book(format!(
                    "The dictionary {} could not be read.",
                    aff.display()
                )));
            }
        };
        Ok(Speller::with_words(dictionary, user_words))
    }

    fn with_words(mut dictionary: Dictionary, user_words: Option<PathBuf>) -> Speller {
        if let Some(path) = &user_words
            && fs::metadata(path).is_ok_and(|meta| meta.len() <= MAX_USER_LIST_BYTES)
            && let Ok(bytes) = fs::read(path)
        {
            for line in String::from_utf8_lossy(&bytes).lines() {
                if is_single_word(line) {
                    let _ = dictionary.add(&plain_form(line));
                }
            }
        }
        Speller {
            dictionary: RwLock::new(dictionary),
            user_words,
        }
    }

    fn dictionary(&self) -> std::sync::RwLockReadGuard<'_, Dictionary> {
        self.dictionary
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn chapter<R: ReadAt>(
        &self,
        reader: &Reader<R>,
        ix: ChapterIx,
    ) -> Result<Vec<Misspelling>, Error> {
        let visible = reader.visible_text(ix)?;
        let spans = misspelled_spans(&self.dictionary(), visible.as_str());
        Ok(spans
            .into_iter()
            .take(MAX_MARKS)
            .map(|span| Misspelling {
                range: visible.to_dom(VisRange {
                    start: span.start as u32,
                    end: span.end as u32,
                }),
            })
            .collect())
    }

    pub fn raw_block(&self, block: BlockIx, text: &str) -> Vec<Misspelling> {
        let spans = misspelled_spans(&self.dictionary(), text);
        let mut units = 0u32;
        let mut seen = 0usize;
        let mut point = |at: usize| {
            units += text[seen..at].encode_utf16().count() as u32;
            seen = at;
            DomPoint {
                block,
                offset_u16: units,
            }
        };
        spans
            .into_iter()
            .take(MAX_MARKS)
            .map(|span| Misspelling {
                range: DomRange {
                    start: point(span.start),
                    end: point(span.end),
                },
            })
            .collect()
    }

    pub fn suggest(&self, word: &str) -> Vec<String> {
        let plain = plain_form(word);
        if !is_single_word(&plain) {
            return Vec::new();
        }
        let dictionary = self.dictionary();
        let mut found = Vec::new();
        let lower = plain.to_lowercase();
        if lower != plain {
            dictionary.suggest(&lower, &mut found);
            found = found.iter().map(|word| capitalized(word)).collect();
        }
        let mut direct = Vec::new();
        dictionary.suggest(&plain, &mut direct);
        for word in direct {
            if !found.contains(&word) {
                found.push(word);
            }
        }
        found.truncate(MAX_SUGGESTIONS);
        found
    }

    pub fn add_word(&self, word: &str) -> Result<(), Error> {
        let plain = plain_form(word);
        if !is_single_word(&plain) {
            return Err(Error::Book(
                "That text is not a single word, so it was not added to the dictionary.".to_owned(),
            ));
        }
        let mut dictionary = self
            .dictionary
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(path) = &self.user_words {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            writeln!(file, "{plain}")?;
        }
        dictionary
            .add(&plain)
            .map_err(|_| Error::Book("That word could not be added to the dictionary.".to_owned()))
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn is_single_word(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_WORD_BYTES
        && text.chars().all(in_word)
        && text.chars().any(char::is_alphabetic)
        && trimmed_word(text, 0..text.len()).is_some_and(|inner| inner == (0..text.len()))
}

#[cfg(test)]
mod tests;
