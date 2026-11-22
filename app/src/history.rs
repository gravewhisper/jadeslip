use crate::library::{RevertedView, reshaped};
use crate::session::{Session, guarded};
use jadeslip_core::{
    DiffPart, EntryName, PointId, PointKind, Reader, RevertKind, RevertTarget, TocSource,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

const MAX_LABEL_CHARS: usize = 300;

fn no_message(error: jadeslip_core::Error) -> String {
    error.user_message()
}

struct Labels<'r> {
    reader: &'r Reader,
    toc: HashMap<u32, String>,
}

impl<'r> Labels<'r> {
    fn new(reader: &'r Reader) -> Labels<'r> {
        let mut toc = HashMap::new();
        if let Ok(found) = reader.toc()
            && !matches!(found.source, TocSource::HeadingsPending)
        {
            for entry in &found.entries {
                if let Some(ix) = entry.target {
                    toc.entry(ix.0)
                        .or_insert_with(|| entry.label.chars().take(MAX_LABEL_CHARS).collect());
                }
            }
        }
        Labels { reader, toc }
    }

    fn of(&self, entry: &EntryName) -> String {
        let file = entry.as_str().rsplit('/').next().unwrap_or(entry.as_str());
        let Some(ix) = self.reader.chapter_of_entry(entry.as_str()) else {
            return file.to_owned();
        };
        if let Some(label) = self.toc.get(&ix.0) {
            return label.clone();
        }
        match self
            .reader
            .rows()
            .get(ix.0 as usize)
            .and_then(|row| row.linear_ordinal)
        {
            Some(number) => format!("Chapter {number}"),
            None => file.to_owned(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointView {
    id: PointId,
    at: u64,
    #[serde(flatten)]
    kind: PointKind,
    chapters: Vec<String>,
}

#[tauri::command(async)]
pub fn history_list(session: State<'_, Session>) -> Result<Vec<PointView>, String> {
    guarded(|| {
        let book = session.book()?;
        let points = book.history().map_err(no_message)?;
        let reader = book.read();
        let labels = Labels::new(&reader);
        Ok(points
            .into_iter()
            .map(|point| PointView {
                id: point.id,
                at: point.at,
                kind: point.kind,
                chapters: point.changed.iter().map(|entry| labels.of(entry)).collect(),
            })
            .collect())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeView {
    entry: String,
    label: String,
    kind: &'static str,
    ix: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesView {
    complete: bool,
    rows: Vec<ChangeView>,
}

#[tauri::command(async)]
pub fn history_changes(session: State<'_, Session>, id: PointId) -> Result<ChangesView, String> {
    guarded(|| {
        let book = session.book()?;
        let changes = book.history_changes(id).map_err(no_message)?;
        let reader = book.read();
        let labels = Labels::new(&reader);
        Ok(ChangesView {
            complete: changes.complete,
            rows: changes
                .rows
                .iter()
                .map(|row| ChangeView {
                    entry: row.entry.as_str().to_owned(),
                    label: labels.of(&row.entry),
                    kind: match row.kind {
                        RevertKind::Changed => "changed",
                        RevertKind::Added => "added",
                        RevertKind::Removed => "removed",
                    },
                    ix: reader.chapter_of_entry(row.entry.as_str()).map(|ix| ix.0),
                })
                .collect(),
        })
    })
}

#[tauri::command(async)]
pub fn history_snapshot(session: State<'_, Session>, name: String) -> Result<PointId, String> {
    guarded(|| {
        let book = session.book()?;
        book.take_snapshot(&name).map_err(no_message)
    })
}

fn entry_name(entry: &str) -> Result<EntryName, String> {
    EntryName::parse(entry).map_err(no_message)
}

#[tauri::command(async)]
pub fn history_diff(
    session: State<'_, Session>,
    id: PointId,
    entry: String,
) -> Result<Vec<DiffPart>, String> {
    guarded(|| {
        let book = session.book()?;
        book.history_diff(id, &entry_name(&entry)?)
            .map_err(no_message)
    })
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RestoreChoice {
    Chapter { entry: String },
    WholeBook,
}

#[tauri::command(async)]
pub fn history_restore(
    session: State<'_, Session>,
    id: PointId,
    choice: RestoreChoice,
    shown: Option<u32>,
) -> Result<RevertedView, String> {
    guarded(|| {
        let book = session.book()?;
        let shown_name = shown.and_then(|ix| book.read().rows().get(ix as usize)?.href.clone());
        let target = match choice {
            RestoreChoice::Chapter { entry } => RevertTarget::Chapter(entry_name(&entry)?),
            RestoreChoice::WholeBook => RevertTarget::WholeBook,
        };
        let report = book.restore_point(id, target).map_err(no_message)?;
        reshaped(&session, &book, shown_name, report)
    })
}
