use crate::session::{Session, data_dir, guarded};
use jadeslip_core::{
    BlockIx, BookKey, ChapterIx, ChapterRev, DomRange, Misspelling, SpellChoice, SpellLanguage,
    Speller, dictionaries_dir, installed, passage_tags, resolve,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::State;

const ENGLISH_WORDS: &str = "spelling-words.txt";

#[derive(Default)]
pub struct Spelling {
    loaded: Mutex<HashMap<SpellLanguage, Result<Arc<Speller>, String>>>,
    told: Mutex<HashSet<SpellLanguage>>,
}

struct Active {
    speller: Option<Arc<Speller>>,
    language: Option<SpellLanguage>,
    notice: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Languages {
    choice: SpellChoice,
    active: Option<SpellLanguage>,
    installed: Vec<String>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Checked {
    Ranges {
        ranges: Vec<DomRange>,
        tags: Vec<String>,
        notice: Option<String>,
    },
    Stale,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn dictionaries() -> Option<PathBuf> {
    data_dir().ok().map(|dir| dictionaries_dir(&dir))
}

fn installed_now() -> Vec<String> {
    dictionaries().map_or_else(Vec::new, |dir| installed(&dir))
}

fn book_state(session: &Session) -> Result<(BookKey, Option<String>, SpellChoice), String> {
    let book = session.book()?;
    let key = BookKey::of(&book.identity());
    let language = book.read().language().map(str::to_owned);
    let choice = session
        .library()
        .map(|library| library.spelling_choice(key))
        .unwrap_or_default();
    Ok((key, language, choice))
}

impl Spelling {
    fn load(language: &SpellLanguage) -> Result<Speller, String> {
        let data = data_dir().ok();
        match language {
            SpellLanguage::English => Ok(Speller::new(data.map(|dir| dir.join(ENGLISH_WORDS)))),
            SpellLanguage::Installed(code) => {
                let data = data.ok_or("The data folder is not set.")?;
                let dir = dictionaries_dir(&data);
                Speller::load(
                    &dir.join(format!("{code}.aff")),
                    &dir.join(format!("{code}.dic")),
                    Some(data.join(format!("spelling-words.{code}.txt"))),
                )
                .map_err(|error| error.user_message())
            }
        }
    }

    fn speller(&self, language: &SpellLanguage) -> Result<Arc<Speller>, String> {
        locked(&self.loaded)
            .entry(language.clone())
            .or_insert_with(|| Spelling::load(language).map(Arc::new))
            .clone()
    }

    fn active(&self, session: &Session) -> Result<Active, String> {
        let (_, book_language, choice) = book_state(session)?;
        let Some(language) = resolve(&choice, book_language.as_deref(), &installed_now()) else {
            return Ok(Active {
                speller: None,
                language: None,
                notice: None,
            });
        };
        match self.speller(&language) {
            Ok(speller) => Ok(Active {
                speller: Some(speller),
                language: Some(language),
                notice: None,
            }),
            Err(why) => {
                let fallback = resolve(&SpellChoice::Auto, book_language.as_deref(), &[]);
                let first = locked(&self.told).insert(language);
                let notice = first.then(|| match fallback {
                    Some(_) => format!("{why} Spelling uses English instead."),
                    None => format!("{why} Spelling marks are off for this book."),
                });
                let speller = fallback
                    .as_ref()
                    .map(|english| self.speller(english))
                    .transpose()?;
                Ok(Active {
                    speller,
                    language: fallback,
                    notice,
                })
            }
        }
    }
}

fn ranges(marks: Vec<Misspelling>) -> Vec<DomRange> {
    marks.into_iter().map(|mark| mark.range).collect()
}

#[tauri::command(async)]
pub fn spell_chapter(
    session: State<'_, Session>,
    spelling: State<'_, Spelling>,
    ix: u32,
    rev: ChapterRev,
) -> Result<Checked, String> {
    #[cfg(feature = "e2e")]
    crate::e2e::spell_delay();
    guarded(|| {
        let reader = session.reader()?;
        if reader.rows().get(ix as usize).and_then(|row| row.rev) != Some(rev) {
            return Ok(Checked::Stale);
        }
        let active = spelling.active(&session)?;
        let marks = match &active.speller {
            Some(speller) => speller
                .chapter(&reader, ChapterIx(ix))
                .map_err(|error| error.user_message())?,
            None => Vec::new(),
        };
        Ok(Checked::Ranges {
            ranges: ranges(marks),
            tags: active.language.as_ref().map_or_else(Vec::new, passage_tags),
            notice: active.notice,
        })
    })
}

#[tauri::command(async)]
pub fn spell_block(
    session: State<'_, Session>,
    spelling: State<'_, Spelling>,
    block: u32,
    text: String,
) -> Result<Vec<DomRange>, String> {
    guarded(|| {
        let active = spelling.active(&session)?;
        Ok(active.speller.map_or_else(Vec::new, |speller| {
            ranges(speller.raw_block(BlockIx(block), &text))
        }))
    })
}

#[tauri::command(async)]
pub fn spell_suggest(
    session: State<'_, Session>,
    spelling: State<'_, Spelling>,
    word: String,
) -> Result<Vec<String>, String> {
    guarded(|| {
        let active = spelling.active(&session)?;
        Ok(active
            .speller
            .map_or_else(Vec::new, |speller| speller.suggest(&word)))
    })
}

#[tauri::command(async)]
pub fn spell_add(
    session: State<'_, Session>,
    spelling: State<'_, Spelling>,
    word: String,
) -> Result<(), String> {
    guarded(|| {
        let speller = spelling
            .active(&session)?
            .speller
            .ok_or("Spelling is not checked in this book, so there is no dictionary to add to.")?;
        speller
            .add_word(&word)
            .map_err(|error| error.user_message())
    })
}

fn languages(session: &Session) -> Result<Languages, String> {
    let (_, book_language, choice) = book_state(session)?;
    let installed = installed_now();
    Ok(Languages {
        active: resolve(&choice, book_language.as_deref(), &installed),
        choice,
        installed,
    })
}

#[tauri::command(async)]
pub fn spell_languages(session: State<'_, Session>) -> Result<Languages, String> {
    guarded(|| languages(&session))
}

#[tauri::command(async)]
pub fn spell_language_set(
    session: State<'_, Session>,
    choice: SpellChoice,
) -> Result<Languages, String> {
    guarded(|| {
        if let SpellChoice::Installed(code) = &choice
            && !installed_now().contains(code)
        {
            return Err(format!("No dictionary named {code} is installed."));
        }
        let (key, _, _) = book_state(&session)?;
        session
            .library()?
            .set_spelling_choice(key, &choice)
            .map_err(|error| error.user_message())?;
        languages(&session)
    })
}
