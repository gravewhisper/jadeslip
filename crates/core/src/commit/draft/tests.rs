use super::*;
use crate::canon::proof::block_xml_of;
use crate::common::fixture;
use crate::edit::BlockChange;
use crate::{BlockIx, BookKey, ChapterRev, DEFAULT_LIMITS};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicU32;

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/draft-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("data")).unwrap();
        fs::copy(fixture("toc-none"), dir.join("book.epub")).unwrap();
        fs::set_permissions(dir.join("book.epub"), fs::Permissions::from_mode(0o644)).unwrap();
        Scratch(dir)
    }

    fn open(&self) -> Book {
        Book::open(
            &self.0.join("book.epub"),
            &self.0.join("data"),
            BookKey(7),
            DEFAULT_LIMITS,
        )
        .unwrap()
    }

    fn book_bytes(&self) -> Vec<u8> {
        fs::read(self.0.join("book.epub")).unwrap()
    }

    fn drafts_dir(&self) -> PathBuf {
        self.0
            .join("data/books")
            .join(format!("{:032x}", 7))
            .join(DRAFTS_DIR)
    }

    fn draft(&self) -> PathBuf {
        let mut found: Vec<PathBuf> = fs::read_dir(self.drafts_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        assert!(found.len() <= 1, "one draft at most: {found:?}");
        found
            .pop()
            .unwrap_or_else(|| self.drafts_dir().join("none.json"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn shown(book: &Book, skip: usize) -> (ChapterIx, ChapterRev, Vec<String>) {
    let reader = book.read();
    reader
        .rows()
        .iter()
        .filter_map(|row| {
            let rendered = reader.render(row.ix).ok()?;
            let xml = block_xml_of(&rendered.xhtml).ok()?;
            (!xml.is_empty()).then_some((row.ix, rendered.rev, xml))
        })
        .nth(skip)
        .expect("the fixture has text")
}

fn typed(book: &Book, letters: &str) -> (ChapterIx, BlockEdits) {
    typed_in(book, letters, 0)
}

fn typed_in(book: &Book, letters: &str, skip: usize) -> (ChapterIx, BlockEdits) {
    let (ix, rev, xml) = shown(book, skip);
    let edited = xml[0].replacen("</", &format!("{letters}</"), 1);
    assert_ne!(edited, xml[0]);
    (
        ix,
        BlockEdits {
            base: rev,
            changes: vec![BlockChange::Replace {
                of: BlockIx(0),
                xml: edited,
            }],
        },
    )
}

#[test]
fn a_draft_comes_back_after_a_reopen_and_recovers_the_same_edits() {
    let scratch = Scratch::new();
    let before = scratch.book_bytes();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.write_draft(ix, &edits).unwrap();
    assert_eq!(scratch.book_bytes(), before, "the book file is untouched");
    assert!(
        scratch.draft().is_file(),
        "the draft sits in the data folder"
    );
    drop(book);

    let reopened = scratch.open();
    let offer = reopened.draft_offer().expect("the draft is offered");
    assert_eq!(offer.fit, DraftFit::Fits);
    assert_eq!(offer.chapter, Some(ix));
    assert!(offer.saved_at.is_some());
    assert_eq!(reopened.recover_draft().unwrap(), (ix, edits.clone()));

    let saved = reopened.save_chapter(ix, &edits).unwrap();
    assert!(matches!(saved.outcome, crate::SaveOutcome::Written(_)));
    assert!(!scratch.draft().exists(), "a save removes the draft");
    assert_eq!(scratch.open().draft_offer(), None);
}

#[test]
fn a_draft_for_a_book_changed_since_is_refused_and_kept_until_discarded() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.write_draft(ix, &edits).unwrap();
    drop(book);

    let other = scratch.open();
    let (next, elsewhere) = typed_in(&other, "vvvv", 1);
    assert_ne!(next, ix, "another chapter changes");
    other.save_chapter(next, &elsewhere).unwrap();
    assert!(
        scratch.draft().exists(),
        "a save does not remove a draft nobody recovered"
    );
    drop(other);

    let reopened = scratch.open();
    let changed = scratch.book_bytes();
    let offer = reopened.draft_offer().expect("the draft is still offered");
    assert_eq!(offer.fit, DraftFit::BookChanged);
    assert!(reopened.recover_draft().is_err());
    assert_eq!(scratch.book_bytes(), changed);
    assert!(scratch.draft().exists());
    reopened.discard_draft().unwrap();
    assert!(!scratch.draft().exists());
    assert_eq!(reopened.draft_offer(), None);
}

#[test]
fn an_unrecovered_draft_is_never_overwritten_by_new_edits() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.write_draft(ix, &edits).unwrap();
    let old = scratch.draft();
    let kept = fs::read(&old).unwrap();
    drop(book);

    let reopened = scratch.open();
    let (_, newer) = typed(&reopened, "vvvv");
    reopened.write_draft(ix, &newer).unwrap();
    reopened.drop_draft().unwrap();
    assert_eq!(fs::read(&old).unwrap(), kept);
    reopened.draft_offer().unwrap();
    assert_eq!(reopened.recover_draft().unwrap(), (ix, edits));
    reopened.write_draft(ix, &newer).unwrap();
    assert_ne!(fs::read(scratch.draft()).unwrap(), kept);
}

#[test]
fn edits_against_an_older_chapter_are_refused() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.save_chapter(ix, &edits).unwrap();
    assert!(matches!(
        book.write_draft(ix, &edits),
        Err(Error::PlanStale)
    ));
    assert!(!scratch.draft().exists());
    let (_, fresh) = typed(&book, "vvvv");
    book.write_draft(ix, &fresh).unwrap();
    drop(book);
    assert_eq!(scratch.open().draft_offer().unwrap().fit, DraftFit::Fits);
}

#[test]
fn no_changes_means_no_draft() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.write_draft(
        ix,
        &BlockEdits {
            base: edits.base,
            changes: Vec::new(),
        },
    )
    .unwrap();
    assert!(!scratch.draft().exists());
    book.write_draft(ix, &edits).unwrap();
    book.write_draft(
        ix,
        &BlockEdits {
            base: edits.base,
            changes: Vec::new(),
        },
    )
    .unwrap();
    assert!(!scratch.draft().exists(), "undoing every edit removes it");
}

#[test]
fn a_write_cut_off_mid_way_leaves_the_previous_draft_whole() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.write_draft(ix, &edits).unwrap();
    let half = scratch.draft().with_extension("json.4242.tmp");
    fs::write(&half, b"{\"version\":1,\"book\":\"ab").unwrap();
    drop(book);

    let reopened = scratch.open();
    assert_eq!(reopened.draft_offer().unwrap().fit, DraftFit::Fits);
    assert!(!half.exists(), "the cut-off temp is swept");
    assert_eq!(reopened.recover_draft().unwrap(), (ix, edits));
}

#[test]
fn a_damaged_draft_is_reported_and_can_be_discarded() {
    let scratch = Scratch::new();
    fs::create_dir_all(scratch.drafts_dir()).unwrap();
    fs::write(scratch.drafts_dir().join("gone.json"), b"not json").unwrap();
    let book = scratch.open();
    let offer = book.draft_offer().unwrap();
    assert_eq!(offer.fit, DraftFit::Unreadable);
    assert_eq!(offer.chapter, None);
    assert!(book.recover_draft().is_err());
    book.discard_draft().unwrap();
    assert_eq!(book.draft_offer(), None);
}

#[test]
fn the_cached_hash_follows_a_save_so_the_next_draft_fits_the_new_file() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.write_draft(ix, &edits).unwrap();
    book.save_chapter(ix, &edits).unwrap();
    let (_, next) = typed(&book, "vvvv");
    book.write_draft(ix, &next).unwrap();
    drop(book);
    let reopened = scratch.open();
    assert_eq!(reopened.draft_offer().unwrap().fit, DraftFit::Fits);
    assert_eq!(reopened.recover_draft().unwrap(), (ix, next));
}

#[test]
fn a_failed_write_leaves_the_previous_draft_whole() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    book.write_draft(ix, &edits).unwrap();
    let kept = fs::read(scratch.draft()).unwrap();
    let (_, newer) = typed(&book, "vvvv");
    crate::commit::seam::install(|step| match step {
        crate::commit::seam::Step::LibrarySynced => Err(io::Error::other("the disk is full")),
        _ => Ok(()),
    });
    let written = book.write_draft(ix, &newer);
    crate::commit::seam::install(|_| Ok(()));
    assert!(written.is_err());
    assert_eq!(fs::read(scratch.draft()).unwrap(), kept);
}

#[test]
fn a_draft_naming_another_chapter_of_the_same_file_is_refused() {
    let scratch = Scratch::new();
    let book = scratch.open();
    let (ix, edits) = typed(&book, "qzjx");
    let (other, _) = typed_in(&book, "vvvv", 1);
    book.write_draft(ix, &edits).unwrap();
    drop(book);
    let text = fs::read_to_string(scratch.draft()).unwrap();
    let moved = text.replacen(
        &format!("\"chapter\":{}", ix.0),
        &format!("\"chapter\":{}", other.0),
        1,
    );
    assert_ne!(moved, text);
    fs::write(scratch.draft(), moved).unwrap();
    let reopened = scratch.open();
    assert_eq!(reopened.draft_offer().unwrap().fit, DraftFit::BookChanged);
    assert!(reopened.recover_draft().is_err());
}

#[test]
fn a_second_window_typing_does_not_replace_the_first_windows_draft() {
    let scratch = Scratch::new();
    let first = scratch.open();
    let second = scratch.open();
    let (other, theirs) = typed_in(&second, "vvvv", 1);
    second.write_draft(other, &theirs).unwrap();
    second.drop_draft().unwrap();
    let (ix, mine) = typed(&first, "qzjx");
    first.write_draft(ix, &mine).unwrap();
    let _ = second.write_draft(other, &theirs);
    drop(first);
    drop(second);

    let reopened = scratch.open();
    let mut recovered = Vec::new();
    while let Some(offer) = reopened.draft_offer() {
        assert_eq!(offer.fit, DraftFit::Fits);
        recovered.push(reopened.recover_draft().unwrap());
        assert!(recovered.len() <= 2);
    }
    assert!(
        recovered.contains(&(ix, mine)),
        "the first window's unsaved edits come back after a crash"
    );
    assert!(
        recovered.contains(&(other, theirs)),
        "and so do the second's"
    );
}

#[test]
fn a_draft_of_a_window_still_open_is_never_offered_or_discarded_elsewhere() {
    let scratch = Scratch::new();
    let first = scratch.open();
    let (ix, mine) = typed(&first, "qzjx");
    first.write_draft(ix, &mine).unwrap();
    let kept = fs::read(scratch.draft()).unwrap();

    let second = scratch.open();
    assert_eq!(second.draft_offer(), None);
    second.discard_draft().unwrap();
    assert_eq!(fs::read(scratch.draft()).unwrap(), kept);
    drop(first);
    assert_eq!(second.draft_offer().unwrap().fit, DraftFit::Fits);
    assert_eq!(second.recover_draft().unwrap(), (ix, mine));
}

#[test]
fn a_second_window_leaving_edit_mode_does_not_delete_the_first_windows_draft() {
    let scratch = Scratch::new();
    let first = scratch.open();
    let second = scratch.open();
    let (other, theirs) = typed_in(&second, "vvvv", 1);
    second.write_draft(other, &theirs).unwrap();
    second.drop_draft().unwrap();
    let (ix, mine) = typed(&first, "qzjx");
    first.write_draft(ix, &mine).unwrap();
    second.drop_draft().unwrap();
    drop(first);
    drop(second);

    let reopened = scratch.open();
    assert_eq!(reopened.draft_offer().unwrap().fit, DraftFit::Fits);
    assert_eq!(
        reopened.recover_draft().unwrap(),
        (ix, mine),
        "the first window's unsaved edits come back after a crash"
    );
}
