mod anchor;

#[cfg(not(windows))]
pub const BOOK_URL_PREFIX: &str = "jadeslip-book://";
#[cfg(windows)]
pub const BOOK_URL_PREFIX: &str = "http://jadeslip-book.localhost/";

mod backup;
mod book;
mod canon;
mod chapter;
mod clean;
pub mod cli;
mod commit;
mod cover;
mod css;
mod edit;
mod entities;
mod history;
mod library;
mod manage;
mod markup;
mod package;
mod prefs;
mod replace;
mod sanitize;
mod search;
mod searches;
mod spell;
pub mod storage;
mod temp;
mod trash;
mod worker;
mod zip;

#[cfg(test)]
mod proofs;

#[cfg(test)]
mod proofs_render;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod common;

pub use anchor::{Anchor, AnchorResolution};
pub use backup::{BackupStatus, RevertTarget, resolve_identity};
pub use book::{BookSummary, ChapterRow, Extracted, LinkTarget, Reader, RenderedChapter, Resource};
pub use canon::proof;
pub use chapter::VisibleText;
pub use clean::{CleanRule, Matcher, Pattern, RULES, RuleSpec};
pub use cover::{CoverFormat, CoverRef};
pub use commit::{
    Book, CommitPhases, CommitReport, DraftFit, DraftOffer, HistoryPoint, PointChanges,
};
pub use edit::{AfterBlock, BlockChange, BlockEdits, BlockOrigin, SaveOutcome, SavedChapter};
pub use history::{DiffKind, DiffPart, PointId, PointKind, Pruned};
pub use library::{
    AddOutcome, CoverFound, CoverStamp, Library, LibraryConfig, Mark, Migration, Removal,
    ResumePoint, ShelfAdd, ShelfEntry,
    WordStamp, count_words, key_hex, parse_key,
};
pub use temp::TempFile;
pub use trash::Trash;
pub use manage::{
    ChapterOp, DeletedChapter, DeletedId, Managed, NewBookMeta, RowRef, Spot, check_new_book,
    new_book,
};
pub use package::{Toc, TocEntry, TocSource};
pub use prefs::{
    Keep, MAX_FONT_PX, MIN_FONT_PX, Pages, Prefs, ReadingFont, ShelfSort, ShelfView, Store, Theme,
};
pub use replace::{
    AppliedReplace, CaseVariant, FixOffer, ReplaceExclusion, ReplacePreview, ReplaceRequest,
    RuleTally, TouchedChapter,
};
pub use sanitize::ChapterKind;
pub use search::{
    ByteBudget, CHAPTER_BYTE_BUDGET, Case, ChapterDelay, DEFAULT_MEMORY_CAP, Hit, Query, SearchEnd,
    SearchEvent, SearchOptions, SearchScope,
};
pub use searches::{
    MAX_RECENT_SEARCHES, MAX_SAVED_SEARCHES, MAX_SEARCH_NAME_CHARS, SavedSearch, SearchMemory,
    SearchQuery,
};
pub use spell::dictionaries::{
    SpellChoice, SpellLanguage, dictionaries_dir, installed, passage_tags, resolve,
};
pub use spell::{Misspelling, Speller};
pub use worker::{PatternEngine, probe_worker, run_worker};
pub use zip::{EntryName, NewEntry, ReadAt, RecordingReader, RevertKind, RevertRow};

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Limits {
    pub max_entries: usize,
    pub max_central_dir: u64,
    pub max_entry_inflated: u64,
    pub max_book_inflated: u64,
    pub max_xml_depth: u32,
    pub max_attrs_per_tag: u32,
    pub max_tag_bytes: u32,
    pub max_markup_events: u32,
}

pub const DEFAULT_LIMITS: Limits = Limits {
    max_entries: 200_000,
    max_central_dir: 64 << 20,
    max_entry_inflated: 64 << 20,
    max_book_inflated: 4 << 30,
    max_xml_depth: 128,
    max_attrs_per_tag: 256,
    max_tag_bytes: 1 << 20,
    max_markup_events: 1_000_000,
};

impl Default for Limits {
    fn default() -> Self {
        DEFAULT_LIMITS
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ChapterIx(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockIx(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ChapterRev {
    pub crc32: u32,
    pub inflated_len: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Fingerprint(pub [u8; 32]);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct BookKey(pub u128);

impl BookKey {
    pub fn of(fingerprint: &Fingerprint) -> BookKey {
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&fingerprint.0[..16]);
        BookKey(u128::from_be_bytes(bytes))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileHash(pub [u8; 32]);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct VisRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DomPoint {
    pub block: BlockIx,
    pub offset_u16: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DomRange {
    pub start: DomPoint,
    pub end: DomPoint,
}

#[derive(Debug)]
pub enum Error {
    ChangedOnDisk,
    PlanStale,
    Cancelled,
    NotEditable(String),
    WouldBreak(String),
    Book(String),
    Limit(String),
    SaveFailed(String),
    Io(std::io::Error),
}

impl Error {
    pub fn user_message(&self) -> String {
        match self {
            Error::ChangedOnDisk => "The book changed on disk. Reload it to continue.".to_owned(),
            Error::PlanStale => {
                "The book changed since this was opened or previewed. Reload it to continue."
                    .to_owned()
            }
            Error::Cancelled => "Stopped. The book was not changed.".to_owned(),
            Error::NotEditable(why)
            | Error::WouldBreak(why)
            | Error::Book(why)
            | Error::Limit(why)
            | Error::SaveFailed(why) => why.clone(),
            Error::Io(error) => format!("The book could not be read. {}", plain_io(error)),
        }
    }
}

pub fn plain_io(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => "The file is not there.",
        ErrorKind::PermissionDenied => "Jadeslip is not allowed to use that file or folder.",
        ErrorKind::ReadOnlyFilesystem => "The disk is read-only.",
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded | ErrorKind::FileTooLarge => {
            "The disk is full."
        }
        ErrorKind::IsADirectory => "That is a folder, not a file.",
        ErrorKind::NotADirectory => "Part of the path is a file, not a folder.",
        ErrorKind::UnexpectedEof | ErrorKind::InvalidData => "The file is cut short or damaged.",
        ErrorKind::Interrupted | ErrorKind::TimedOut | ErrorKind::WouldBlock => {
            "The disk did not answer in time."
        }
        _ => "The system could not read or write the file.",
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

#[cfg(test)]
mod plain_io_tests {
    use super::*;

    #[test]
    fn no_io_failure_shows_an_os_error_number() {
        for code in 1..=133 {
            let message = Error::Io(std::io::Error::from_raw_os_error(code)).user_message();
            assert!(!message.contains("os error"), "{code}: {message}");
            assert!(message.ends_with('.'), "{code}: {message}");
        }
        let missing = Error::Io(std::io::ErrorKind::NotFound.into()).user_message();
        assert_eq!(missing, "The book could not be read. The file is not there.");
    }
}

