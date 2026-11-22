use super::*;
use crate::common::fixture;
use crate::manage::{ChapterOp, RowRef, Spot};
use crate::{AnchorResolution, BlockIx, DomRange, Extracted};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/library-tests")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("data")).unwrap();
        Scratch(dir)
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn library(&self) -> Arc<Library> {
        Library::open(LibraryConfig {
            data_dir: self.data(),
        })
        .unwrap()
    }

    fn copy(&self, name: &str, as_name: &str) -> PathBuf {
        let path = self.0.join(as_name);
        fs::copy(fixture(name), &path).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const SOURCE: &str = "toc-nav-only";

fn shelved(library: &Arc<Library>, path: &Path) -> Book {
    let add = library.add_to_shelf(path).unwrap();
    library.open_book(&library.entry(add.key).unwrap().path).unwrap()
}

fn key_of(book: &Book) -> BookKey {
    BookKey::of(&book.identity())
}

fn meta(title: &str) -> NewBookMeta {
    NewBookMeta {
        title: title.to_owned(),
        author: "An Author".to_owned(),
        language: "en".to_owned(),
        first_chapter: "One".to_owned(),
    }
}

fn point(block: u32, offset_u16: u32) -> DomPoint {
    DomPoint {
        block: BlockIx(block),
        offset_u16,
    }
}

fn resume(href: &str, passage: &str) -> ResumePoint {
    ResumePoint {
        href: href.to_owned(),
        point: Some(point(1, 2)),
        chapter_label: "Label".to_owned(),
        ordinal: Some(2),
        progress: 0.25,
        passage: passage.to_owned(),
    }
}

fn first_text_chapter(book: &Book) -> ChapterIx {
    let reader = book.read();
    reader
        .rows()
        .iter()
        .find(|row| {
            !row.is_nav
                && row.linear_ordinal.is_some()
                && reader
                    .visible_text(row.ix)
                    .is_ok_and(|text| text.as_str().len() > 200)
        })
        .map(|row| row.ix)
        .unwrap()
}

fn whole_range_in_second_line(book: &Book, ix: ChapterIx) -> (Anchor, String) {
    let reader = book.read();
    let visible = reader.visible_text(ix).unwrap();
    let text = visible.as_str();
    let start = text.find('\n').unwrap() + 1;
    let start = start + text[start..].find(' ').unwrap() + 1;
    let end = start + text[start..].char_indices().nth(30).unwrap().0;
    let range = visible.to_dom(VisRange {
        start: start as u32,
        end: end as u32,
    });
    let anchor = reader.anchor_at(ix, range).unwrap();
    assert!(anchor.exact.len() >= 16, "the anchor holds text");
    let exact = anchor.exact.clone();
    (anchor, exact)
}

fn text_of(reader: &Reader<File>, ix: ChapterIx, range: VisRange) -> String {
    let visible = reader.visible_text(ix).unwrap();
    visible.as_str()[range.start as usize..range.end as usize].to_owned()
}

fn append_paragraph_before_text(book: &Book, ix: ChapterIx) {
    let reader = book.read();
    let Extracted::Document(bytes) = reader.extract(ix).unwrap() else {
        panic!("a text chapter");
    };
    let source = String::from_utf8(bytes).unwrap();
    let body = source.find("<body").unwrap();
    let close = body + source[body..].find('>').unwrap() + 1;
    let edited = format!(
        "{}<p>A brand new opening line was added here.</p>{}",
        &source[..close],
        &source[close..]
    );
    let name = reader.rows()[ix.0 as usize].href.clone().unwrap();
    book.save_entries(vec![crate::NewEntry::deflate(name, edited.as_bytes())])
        .unwrap();
}

#[test]
fn adding_shelves_a_copy_in_the_library_and_leaves_the_file_alone() {
    let scratch = Scratch::new("record");
    let path = scratch.copy(SOURCE, "book.epub");
    let before = fs::read(&path).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let library = scratch.library();
    let add = library.add_to_shelf(&path).unwrap();
    assert_eq!(add.outcome, AddOutcome::Added);
    let shelf = library.shelf();
    assert_eq!(shelf.len(), 1);
    let (key, entry) = &shelf[0];
    assert_eq!(*key, add.key);
    assert!(library.is_managed(&entry.path), "{}", entry.path.display());
    assert_eq!(
        entry.path.file_name().unwrap().to_string_lossy(),
        format!("Toc Nav Only - {}.epub", &key_hex(*key)[..8])
    );
    assert_eq!(fs::read(&entry.path).unwrap(), before);
    assert_eq!(fs::metadata(&entry.path).unwrap().modified().unwrap(), mtime);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    let book = library.open_book(&entry.path).unwrap();
    assert_eq!(key_of(&book), *key);
    let reader = book.read();
    assert_eq!(entry.title, reader.summary().title);
    assert_eq!(entry.linear_chapters, reader.summary().linear_chapters);
    assert!(entry.inflated_spine_bytes > 0);
    assert!(!entry.own);
    assert_eq!(library.shelf().len(), 1);
}

#[test]
fn opening_in_place_does_not_shelve_but_keeps_the_place() {
    let scratch = Scratch::new("in-place");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let key = key_of(&book);
    assert!(library.shelf().is_empty());
    assert!(!scratch.data().join(LIBRARY_FILE).exists());
    assert!(!library.library_dir().exists());
    library
        .save_resume_point(key, false, resume("a.xhtml", "Read in place."))
        .unwrap();
    drop(book);
    let later = scratch.library();
    assert!(later.shelf().is_empty());
    assert_eq!(later.entry(key), None);
    let known = later.known(key).unwrap();
    assert_eq!(known.path, fs::canonicalize(&path).unwrap());
    assert_eq!(known.read_point.unwrap().passage, "Read in place.");
    later.open_book(&path).unwrap();
    assert!(later.shelf().is_empty());

    let add = later.add_to_shelf(&path).unwrap();
    assert_eq!(add.key, key);
    let entry = later.entry(key).unwrap();
    assert_eq!(entry.read_point.unwrap().passage, "Read in place.", "the place comes along");
    assert!(later.is_managed(&entry.path));
    assert!(!scratch.data().join("books").join(key_hex(key)).join(STASH_FILE).exists());
}

#[test]
fn the_shelf_survives_a_restart_without_touching_any_book() {
    let scratch = Scratch::new("restart");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = key_of(&shelved(&library, &path));
    library
        .save_resume_point(key, false, resume("a.xhtml", "Words here."))
        .unwrap();
    fs::remove_file(&path).unwrap();
    let later = scratch.library();
    let shelf = later.shelf();
    assert_eq!(shelf.len(), 1);
    assert_eq!(
        shelf[0].1.read_point,
        Some(resume("a.xhtml", "Words here."))
    );
    assert!(later.open_book(&path).is_err());
    assert_eq!(later.shelf().len(), 1);
}

#[test]
fn a_package_edit_keeps_the_same_record() {
    let scratch = Scratch::new("identity");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    book.manage(
        ChapterOp::Add {
            after: None,
            title: "Fresh".to_owned(),
        },
        None,
    )
    .unwrap();
    drop(book);
    let reopened = library.open_book(&copy).unwrap();
    assert_eq!(key_of(&reopened), key);
    let shelf = library.shelf();
    assert_eq!(shelf.len(), 1);
    assert_eq!(shelf[0].1.path, copy);
    assert_eq!(
        shelf[0].1.linear_chapters,
        reopened.read().summary().linear_chapters
    );
    assert_ne!(fs::read(&copy).unwrap(), fs::read(&path).unwrap(), "the edit went to the copy");
    assert_eq!(fs::read(&path).unwrap(), fs::read(fixture(SOURCE)).unwrap(), "the added file is untouched");
}

#[test]
fn adding_a_book_already_on_the_shelf_offers_the_copy_and_makes_no_second() {
    let scratch = Scratch::new("twins");
    let one = scratch.copy(SOURCE, "one.epub");
    let two = scratch.copy(SOURCE, "two.epub");
    let library = scratch.library();
    let first = library.add_to_shelf(&one).unwrap();
    let copy = library.entry(first.key).unwrap().path;
    let second = library.add_to_shelf(&two).unwrap();
    assert_eq!(second.key, first.key);
    assert_eq!(second.outcome, AddOutcome::AlreadyOnShelf);
    assert_eq!(second.title, "Toc Nav Only");
    let again = library.add_to_shelf(&copy).unwrap();
    assert_eq!(again.outcome, AddOutcome::AlreadyOnShelf);
    assert_eq!(library.shelf().len(), 1);
    assert_eq!(library.entry(first.key).unwrap().path, copy);
    assert_eq!(fs::read_dir(library.library_dir()).unwrap().count(), 1);

    let newer = scratch.copy("toc-ncx-only", "newer.epub");
    let other = library.add_to_shelf(&newer).unwrap();
    assert_eq!(other.outcome, AddOutcome::Added);
    assert_ne!(other.key, first.key);
    assert_eq!(library.shelf().len(), 2);
}

#[test]
fn a_book_with_the_same_title_and_another_fingerprint_is_a_second_book() {
    let scratch = Scratch::new("versions");
    let one = scratch.copy(SOURCE, "one.epub");
    let library = scratch.library();
    let first = library.add_to_shelf(&one).unwrap();
    let edited = scratch.copy(SOURCE, "edited.epub");
    let book = library.open_book(&edited).unwrap();
    book.manage(
        ChapterOp::Add {
            after: None,
            title: "Later chapter".to_owned(),
        },
        None,
    )
    .unwrap();
    drop(book);
    fs::remove_dir_all(scratch.data().join("aliases")).unwrap();
    let second = library.add_to_shelf(&edited).unwrap();
    assert_eq!(second.outcome, AddOutcome::Added);
    assert_ne!(second.key, first.key);
    let shelf = library.shelf();
    assert_eq!(shelf.len(), 2);
    assert_eq!(shelf[0].1.title, shelf[1].1.title);
    assert_ne!(shelf[0].1.path, shelf[1].1.path);
}

#[test]
fn adding_through_a_symlink_copies_the_book_and_leaves_both_alone() {
    let scratch = Scratch::new("symlink");
    let real = scratch.copy(SOURCE, "real.epub");
    let link = scratch.0.join("link.epub");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let library = scratch.library();
    let add = library.add_to_shelf(&link).unwrap();
    let copy = library.entry(add.key).unwrap().path;
    assert!(library.is_managed(&copy));
    assert!(!fs::symlink_metadata(&copy).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&real).unwrap());
    assert_eq!(fs::read_link(&link).unwrap(), real);
}

#[test]
fn a_new_book_is_made_in_the_library_marked_as_the_owners_and_stays_so() {
    let scratch = Scratch::new("own");
    let library = scratch.library();
    let book = library.create_book(&meta("Mine")).unwrap();
    let key = key_of(&book);
    let entry = library.entry(key).unwrap();
    assert!(entry.own);
    assert_eq!(entry.path, book.path());
    assert!(library.is_managed(book.path()));
    assert_eq!(
        book.path().file_name().unwrap().to_string_lossy(),
        format!("Mine - {}.epub", &key_hex(key)[..8])
    );
    assert_eq!(fs::read_dir(library.library_dir()).unwrap().count(), 1, "no stage file is left");
    let path = book.path().to_owned();
    drop(book);
    library.open_book(&path).unwrap();
    assert!(library.entry(key).unwrap().own);
    library.set_own(key, false).unwrap();
    assert!(!scratch.library().entry(key).unwrap().own);
}

#[test]
fn a_bad_record_keeps_the_others_and_a_damaged_file_is_set_aside() {
    let scratch = Scratch::new("damage");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = key_of(&shelved(&library, &path));
    let file = scratch.data().join(LIBRARY_FILE);
    let mut root: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    root["books"]["not a key"] = serde_json::json!({"title": 7});
    root["books"][format!("{:032x}", 1)] = serde_json::json!({"title": 7});
    fs::write(&file, serde_json::to_vec(&root).unwrap()).unwrap();
    let shelf = scratch.library().shelf();
    assert_eq!(shelf.len(), 1);
    assert_eq!(shelf[0].0, key);

    let held_aside = |dir: PathBuf| {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".damaged-"))
            .count()
    };
    let before = held_aside(scratch.data());
    std::thread::sleep(std::time::Duration::from_millis(2));
    fs::write(&file, b"\xff not json").unwrap();
    let fresh = scratch.library();
    assert!(fresh.shelf().is_empty());
    assert_eq!(
        held_aside(scratch.data()),
        before + 1,
        "the damaged file is kept beside the new one"
    );
}

#[test]
fn a_record_that_will_not_parse_is_kept_aside_before_it_is_dropped() {
    let scratch = Scratch::new("kept");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = key_of(&shelved(&library, &path));
    let file = scratch.data().join(LIBRARY_FILE);
    let mut root: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    root["books"][format!("{:032x}", 1)] = serde_json::json!({"title": 7});
    fs::write(&file, serde_json::to_vec(&root).unwrap()).unwrap();
    library
        .save_resume_point(key, false, resume("a.xhtml", "Kept."))
        .unwrap();
    let kept = fs::read_dir(scratch.data())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".damaged-"))
        .count();
    assert_eq!(kept, 1);
}

#[test]
fn a_file_name_that_is_not_utf8_opens_in_place_and_comes_back_exactly() {
    use std::os::unix::ffi::OsStringExt;
    let scratch = Scratch::new("raw-name");
    let source = scratch.copy(SOURCE, "plain.epub");
    let name = std::ffi::OsString::from_vec(b"\xC4\xE3\xBA\xC3.epub".to_vec());
    let path = scratch.0.join(name);
    fs::rename(&source, &path).unwrap();
    let library = scratch.library();
    let key = key_of(&library.open_book(&path).unwrap());
    let known = scratch.library().known(key).unwrap();
    assert_eq!(known.path, fs::canonicalize(&path).unwrap());
    assert!(library.open_book(&known.path).is_ok());
    let add = library.add_to_shelf(&path).unwrap();
    assert!(library.entry(add.key).unwrap().path.to_str().is_some());
}

#[test]
fn a_write_keeps_a_record_another_window_added() {
    let scratch = Scratch::new("windows");
    let one = scratch.copy(SOURCE, "one.epub");
    let two = scratch.copy("toc-ncx-only", "two.epub");
    let first = scratch.library();
    let second = scratch.library();
    let key = key_of(&shelved(&first, &one));
    second.add_to_shelf(&two).unwrap();
    first
        .save_resume_point(key, false, resume("a.xhtml", "Kept."))
        .unwrap();
    assert_eq!(scratch.library().shelf().len(), 2);
}

#[test]
fn removing_a_record_keeps_the_book_file_and_the_marks() {
    let scratch = Scratch::new("remove");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    let ix = first_text_chapter(&book);
    let (anchor, _) = whole_range_in_second_line(&book, ix);
    library.add_mark(key, anchor, None).unwrap();
    library.remove(key).unwrap();
    assert!(library.shelf().is_empty());
    assert!(path.exists());
    assert!(copy.exists(), "Jadeslip's copy stays unless asked");
    assert_eq!(library.marks(key).unwrap().len(), 1);
}

#[test]
fn reading_and_writing_points_are_kept_apart() {
    let scratch = Scratch::new("points");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = key_of(&shelved(&library, &path));
    library
        .save_resume_point(key, true, resume("w.xhtml", "Writing."))
        .unwrap();
    library
        .save_resume_point(key, false, resume("r.xhtml", "Reading."))
        .unwrap();
    let entry = library.entry(key).unwrap();
    assert_eq!(entry.write_point.unwrap().href, "w.xhtml");
    assert_eq!(entry.read_point.unwrap().href, "r.xhtml");
}

#[test]
fn a_resume_point_names_the_place_with_a_passage_and_progress() {
    let scratch = Scratch::new("resume");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let ix = first_text_chapter(&book);
    let reader = book.read();
    let top = reader.resume_point(ix, None).unwrap();
    assert!(!top.passage.is_empty());
    assert!(!top.chapter_label.is_empty());
    assert!(top.progress < 1.0);
    let visible = reader.visible_text(ix).unwrap();
    let late = visible.to_dom(VisRange {
        start: visible.as_str().len() as u32 / 2,
        end: visible.as_str().len() as u32 / 2,
    });
    let halfway = reader.resume_point(ix, Some(late.start)).unwrap();
    assert!(halfway.progress > top.progress);
    assert!(halfway.passage != top.passage);
    let wild = reader
        .resume_point(ix, Some(point(9_999_999, 9_999_999)))
        .unwrap();
    assert!(wild.progress <= 1.0);
    let far = reader
        .anchor_at(
            ix,
            DomRange {
                start: point(9_999_999, 1),
                end: point(9_999_999, 9),
            },
        )
        .unwrap();
    assert!(far.exact.len() < MAX_NOTE_CHARS);
}

#[test]
fn a_passage_is_flat_short_and_ends_cleanly() {
    let long = format!("Short title\n{}", "One sentence goes here. ".repeat(40));
    let passage = passage_at(&long, 0, true);
    assert!(!passage.contains('\n'));
    assert!(!passage.starts_with("Short title"));
    assert!(passage.chars().count() <= PASSAGE_CHARS);
    assert!(passage.ends_with('.'));
    let unbroken = "word ".repeat(200);
    assert!(passage_at(&unbroken, 0, false).ends_with("word"));
    assert_eq!(passage_at("", 0, true), "");
    assert_eq!(passage_at("Only a title", 0, true), "Only a title");
    let _ = passage_at("ééé", 1, false);
    let _ = passage_at("ééé", 500, false);
}

#[test]
fn marks_keep_their_place_and_never_touch_the_book() {
    let scratch = Scratch::new("marks");
    let path = scratch.copy(SOURCE, "book.epub");
    let before = fs::read(&path).unwrap();
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let key = key_of(&book);
    let ix = first_text_chapter(&book);
    let (anchor, _) = whole_range_in_second_line(&book, ix);
    let one = library
        .add_mark(key, anchor.clone(), Some("  a note  ".to_owned()))
        .unwrap();
    let two = library.add_mark(key, anchor, None).unwrap();
    assert_eq!(one.note.as_deref(), Some("a note"));
    assert_eq!(two.note, None);
    assert_ne!(one.id, two.id);
    assert_eq!(fs::read(&path).unwrap(), before);
    let file = scratch
        .data()
        .join("books")
        .join(key_hex(key))
        .join(MARKS_FILE);
    assert!(file.is_file());
    library.delete_mark(key, one.id).unwrap();
    let left = scratch.library().marks(key).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, two.id);
}

#[test]
fn a_damaged_marks_file_is_refused_and_left_in_place() {
    let scratch = Scratch::new("marks-bad");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let key = key_of(&book);
    let ix = first_text_chapter(&book);
    let (anchor, _) = whole_range_in_second_line(&book, ix);
    library.add_mark(key, anchor.clone(), None).unwrap();
    let file = scratch
        .data()
        .join("books")
        .join(key_hex(key))
        .join(MARKS_FILE);
    fs::write(&file, b"{broken").unwrap();
    assert!(library.marks(key).is_err());
    assert!(library.add_mark(key, anchor, None).is_err());
    assert!(library.delete_mark(key, 1).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"{broken");
}

#[test]
fn a_mark_survives_an_edit_a_move_a_delete_and_an_undo() {
    let scratch = Scratch::new("survive");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let ix = first_text_chapter(&book);
    let (anchor, exact) = whole_range_in_second_line(&book, ix);
    assert_eq!(
        book.read().resolve(&anchor),
        AnchorResolution::Exact {
            chapter: ix,
            range: match book.read().resolve(&anchor) {
                AnchorResolution::Exact { range, .. } => range,
                other => panic!("not exact: {other:?}"),
            }
        }
    );

    append_paragraph_before_text(&book, ix);
    let found = book.read().resolve(&anchor);
    let AnchorResolution::Moved { chapter, range } = found else {
        panic!("an edit before the mark reports moved: {found:?}");
    };
    assert_eq!(chapter, ix);
    assert_eq!(text_of(&book.read(), chapter, range), exact);

    let last = book.read().rows().len() as u32 - 1;
    book.manage(
        ChapterOp::Move {
            row: RowRef::Chapter(ix),
            to: Spot {
                row: RowRef::Chapter(ChapterIx(last)),
                after: true,
            },
        },
        None,
    )
    .unwrap();
    let reader = book.read();
    let found = reader.resolve(&anchor);
    let AnchorResolution::Moved { chapter, range } = found else {
        panic!("a reorder still finds the text: {found:?}");
    };
    assert_ne!(chapter, ix, "the chapter has a new place in the spine");
    assert_eq!(text_of(&reader, chapter, range), exact);
    drop(reader);

    let managed = book.manage(ChapterOp::Delete { chapter }, None).unwrap();
    assert_eq!(
        book.read().resolve(&anchor),
        AnchorResolution::Detached,
        "with its chapter gone the mark is detached"
    );
    book.manage(
        ChapterOp::UndoDelete {
            deleted: managed.deleted.unwrap(),
        },
        None,
    )
    .unwrap();
    let reader = book.read();
    let AnchorResolution::Moved { chapter, range } = reader.resolve(&anchor) else {
        panic!("an undo brings the mark back");
    };
    assert_eq!(text_of(&reader, chapter, range), exact);
}

#[test]
fn a_mark_whose_file_is_gone_is_found_by_its_text_elsewhere() {
    let scratch = Scratch::new("elsewhere");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let ix = first_text_chapter(&book);
    let (mut anchor, exact) = whole_range_in_second_line(&book, ix);
    anchor.href = "no/such/file.xhtml".to_owned();
    let reader = book.read();
    let AnchorResolution::Moved { chapter, range } = reader.resolve(&anchor) else {
        panic!("the whole book is scanned");
    };
    assert_eq!(chapter, ix);
    assert_eq!(text_of(&reader, chapter, range), exact);
    let mut gone = anchor.clone();
    gone.exact = "text that is nowhere in this generated book at all".to_owned();
    let both = reader.resolve_all(&[gone, anchor]);
    assert_eq!(both[0], AnchorResolution::Detached);
    assert!(matches!(both[1], AnchorResolution::Moved { .. }));
}

#[test]
fn anchors_map_a_dom_range_through_the_text_model() {
    let scratch = Scratch::new("dom");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let ix = first_text_chapter(&book);
    let reader = book.read();
    let collapsed = reader
        .anchor_at(
            ix,
            DomRange {
                start: point(1, 0),
                end: point(1, 0),
            },
        )
        .unwrap();
    assert!(!collapsed.exact.is_empty());
    assert!(
        reader
            .anchor_at(
                ChapterIx(9_999),
                DomRange {
                    start: point(0, 0),
                    end: point(0, 0)
                }
            )
            .is_err()
    );
}

#[test]
fn a_blocked_data_folder_still_opens_the_book() {
    let scratch = Scratch::new("blocked");
    let path = scratch.copy(SOURCE, "book.epub");
    let blocked = scratch.0.join("blocked-file");
    fs::write(&blocked, b"a file where the folder should be").unwrap();
    let library = Library::open(LibraryConfig {
        data_dir: blocked.clone(),
    })
    .unwrap();
    let book = library.open_book(&path).unwrap();
    assert!(book.read().summary().linear_chapters > 0);
    assert!(
        library
            .save_resume_point(key_of(&book), false, resume("a.xhtml", "x"))
            .is_err()
    );
}

#[test]
fn keys_round_trip_and_junk_is_refused() {
    let key = BookKey(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    assert_eq!(parse_key(&key_hex(key)), Some(key));
    assert_eq!(parse_key("zz"), None);
    assert_eq!(parse_key(&"z".repeat(32)), None);
    assert_eq!(parse_key(&"é".repeat(16)), None);
}

#[test]
fn an_export_is_byte_equal_and_never_replaces_the_book() {
    let scratch = Scratch::new("export");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = library.open_book(&path).unwrap();
    let copy = scratch.0.join("copy.epub");
    book.export_copy(&copy).unwrap();
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&path).unwrap());
    assert!(book.export_copy(&path).is_err());
    let leftovers = fs::read_dir(&scratch.0)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0);
    book.export_copy(&copy).unwrap();
    assert!(
        book.export_copy(&scratch.0.join("no/such/dir/copy.epub"))
            .is_err()
    );
}

#[test]
fn a_saved_place_is_synced_before_it_replaces_the_old_and_its_folder_after() {
    use crate::commit::seam::{Step, install};
    use std::cell::RefCell;
    use std::rc::Rc;
    let scratch = Scratch::new("durable-place");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = key_of(&shelved(&library, &path));
    let shelf_file = scratch.data().join(LIBRARY_FILE);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    install(move |step| {
        let on_disk = fs::read_to_string(&shelf_file).unwrap();
        log.borrow_mut()
            .push((step, on_disk.contains("Synced place.")));
        Ok(())
    });
    library
        .save_resume_point(key, false, resume("a.xhtml", "Synced place."))
        .unwrap();
    install(|_| Ok(()));
    assert_eq!(
        *seen.borrow(),
        [(Step::LibrarySynced, false), (Step::LibraryDirSynced, true)]
    );
}

#[test]
fn a_spelling_choice_is_kept_per_book_and_a_damaged_one_reads_as_auto() {
    let scratch = Scratch::new("spelling-choice");
    let one = BookKey(1);
    let two = BookKey(2);
    let library = scratch.library();
    assert_eq!(library.spelling_choice(one), SpellChoice::Auto);
    library
        .set_spelling_choice(one, &SpellChoice::Installed("xx".to_owned()))
        .unwrap();
    library
        .set_spelling_choice(two, &SpellChoice::English)
        .unwrap();
    let again = scratch.library();
    assert_eq!(
        again.spelling_choice(one),
        SpellChoice::Installed("xx".to_owned())
    );
    assert_eq!(again.spelling_choice(two), SpellChoice::English);
    for bytes in [
        &b"{not json"[..],
        br#"{"version":9,"choice":{"kind":"english"}}"#,
        br#"{"version":1,"choice":{"kind":"martian"}}"#,
    ] {
        fs::write(again.spelling_path(one), bytes).unwrap();
        assert_eq!(again.spelling_choice(one), SpellChoice::Auto);
    }
}

fn thumbnail_dimensions(bytes: &[u8]) -> (u32, u32) {
    let picture = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).unwrap();
    (picture.width(), picture.height())
}

fn damaged_copies(dir: &Path) -> usize {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".damaged-"))
        .count()
}

#[test]
fn a_shelf_written_before_covers_reads_with_nothing_lost() {
    let scratch = Scratch::new("old-shelf");
    let path = scratch.copy("cover-epub3", "book.epub");
    let library = scratch.library();
    let key = key_of(&shelved(&library, &path));
    let file = scratch.data().join(LIBRARY_FILE);
    let mut root: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    let record = root["books"][key_hex(key)].as_object_mut().unwrap();
    assert!(record.remove("cover").is_some());
    fs::write(&file, serde_json::to_vec(&root).unwrap()).unwrap();
    let later = scratch.library();
    assert_eq!(later.entry(key).unwrap().cover, None);
    assert_eq!(damaged_copies(&scratch.data()), 0);
}

#[test]
fn a_cover_is_stamped_once_and_served_from_the_data_folder() {
    let scratch = Scratch::new("cover");
    let path = scratch.copy("cover-epub3", "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    assert_eq!(library.cover_bytes(key), None);
    assert!(library.update_cover(key, &book.read()).unwrap());
    let stamp = library.entry(key).unwrap().cover.unwrap();
    assert_eq!(stamp.found, CoverFound::Thumbnail);
    let (width, height) = thumbnail_dimensions(&library.cover_bytes(key).unwrap());
    assert!(width <= 400 && height <= 600 && width > 0);
    assert!(
        !library.update_cover(key, &book.read()).unwrap(),
        "same file, no second read"
    );
    assert_eq!(
        scratch.library().entry(key).unwrap().cover,
        Some(stamp.clone())
    );

    fs::File::options()
        .write(true)
        .open(&copy)
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000))
        .unwrap();
    let reopened = library.open_book(&copy).unwrap();
    assert!(
        library.update_cover(key, &reopened.read()).unwrap(),
        "a changed file is read again"
    );
    let restamped = library.entry(key).unwrap().cover.unwrap();
    assert_ne!(restamped.file, stamp.file);
    assert_eq!(restamped.found, CoverFound::Thumbnail);
}

#[test]
fn a_book_without_a_cover_is_stamped_absent() {
    let scratch = Scratch::new("no-cover");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let key = key_of(&book);
    assert!(library.update_cover(key, &book.read()).unwrap());
    assert_eq!(
        library.entry(key).unwrap().cover.unwrap().found,
        CoverFound::Absent
    );
    assert_eq!(library.cover_bytes(key), None);
}

#[test]
fn filling_reads_only_unstamped_books_that_are_still_there() {
    let scratch = Scratch::new("fill");
    let three = scratch.copy("cover-epub3", "three.epub");
    let two = scratch.copy("cover-epub2", "two.epub");
    let plain = scratch.copy(SOURCE, "plain.epub");
    let gone = scratch.copy("toc-ncx-only", "gone.epub");
    let library = scratch.library();
    let keys: Vec<BookKey> = [&three, &two, &plain, &gone]
        .iter()
        .map(|path| library.add_to_shelf(path).unwrap().key)
        .collect();
    fs::remove_file(library.entry(keys[3]).unwrap().path).unwrap();
    assert_eq!(library.fill_covers(), 3);
    let found = |key: BookKey| library.entry(key).unwrap().cover.map(|cover| cover.found);
    assert_eq!(found(keys[0]), Some(CoverFound::Thumbnail));
    assert_eq!(found(keys[1]), Some(CoverFound::Thumbnail));
    assert_eq!(found(keys[2]), Some(CoverFound::Absent));
    assert_eq!(found(keys[3]), None, "a missing file is left for later");
    assert!(library.cover_bytes(keys[1]).is_some());
    assert_eq!(library.fill_covers(), 0, "stamped books are not read again");
}

#[test]
fn a_record_whose_file_now_holds_another_book_gets_no_cover() {
    let scratch = Scratch::new("swapped");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = key_of(&shelved(&library, &path));
    fs::copy(fixture("cover-epub3"), library.entry(key).unwrap().path).unwrap();
    assert_eq!(library.fill_covers(), 1);
    assert_eq!(
        library.entry(key).unwrap().cover.unwrap().found,
        CoverFound::Absent
    );
    assert_eq!(library.cover_bytes(key), None);
}

#[test]
fn words_are_runs_between_spaces_as_in_the_editor() {
    assert_eq!(count_words(""), 0);
    assert_eq!(count_words("   "), 0);
    assert_eq!(count_words("One"), 1);
    assert_eq!(count_words("  Han Mo smiled.\n\nThe end "), 5);
    assert_eq!(count_words("non\u{a0}breaking\u{2009}thin\u{3000}ideographic"), 4);
    assert_eq!(count_words("don't stop\u{2014}now"), 2);
}

#[test]
fn an_own_book_counts_its_words_once_per_file_version() {
    let scratch = Scratch::new("words");
    let library = scratch.library();
    let book = library.create_book(&meta("Mine")).unwrap();
    let key = key_of(&book);
    assert!(library.update_words(key, &book.read()).unwrap());
    let first = library.entry(key).unwrap().words.unwrap();
    assert_eq!(first.count, book.read().word_count());
    assert!(!library.update_words(key, &book.read()).unwrap());
    let ix = book
        .read()
        .rows()
        .iter()
        .find(|row| row.linear_ordinal.is_some())
        .unwrap()
        .ix;
    append_paragraph_before_text(&book, ix);
    assert!(library.update_words(key, &book.read()).unwrap());
    let second = library.entry(key).unwrap().words.unwrap();
    assert_ne!(second.file, first.file);
    assert!(second.count > first.count, "{second:?} {first:?}");
}

#[test]
fn a_stale_reader_does_not_overwrite_a_newer_count_and_a_read_book_is_not_counted() {
    let scratch = Scratch::new("stale");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let key = key_of(&book);
    assert!(!library.update_words(key, &book.read()).unwrap());
    assert_eq!(library.entry(key).unwrap().words, None);
    library.set_own(key, true).unwrap();
    let old = book.read();
    append_paragraph_before_text(&book, first_text_chapter(&book));
    assert!(!library.update_words(key, &old).unwrap());
    assert!(library.update_words_from_file(key).unwrap());
    assert_eq!(
        library.entry(key).unwrap().words.unwrap().count,
        book.read().word_count()
    );
}

#[test]
fn a_removed_book_comes_back_with_its_place_marks_and_its_kept_copy() {
    let scratch = Scratch::new("readd");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    library
        .save_resume_point(key, false, resume("r.xhtml", "Where I was."))
        .unwrap();
    library.set_own(key, true).unwrap();
    let ix = first_text_chapter(&book);
    let (anchor, _) = whole_range_in_second_line(&book, ix);
    library.add_mark(key, anchor, None).unwrap();
    append_paragraph_before_text(&book, ix);
    let edited = fs::read(&copy).unwrap();
    library.remove(key).unwrap();
    assert!(library.shelf().is_empty());
    drop(book);
    library.open_book(&path).unwrap();
    assert!(library.shelf().is_empty(), "opening the original in place does not shelve it");
    let add = library.add_to_shelf(&path).unwrap();
    assert_eq!(add.outcome, AddOutcome::Added);
    let entry = library.entry(key).unwrap();
    assert_eq!(entry.read_point.unwrap().passage, "Where I was.");
    assert!(entry.own);
    assert_eq!(entry.path, copy, "the kept copy comes back");
    assert_eq!(fs::read(&copy).unwrap(), edited, "with the edits made before removing");
    assert_eq!(fs::read_dir(library.library_dir()).unwrap().count(), 1);
    assert_eq!(library.marks(key).unwrap().len(), 1);
    assert!(!scratch.data().join("books").join(key_hex(key)).join(STASH_FILE).exists());
}

fn trash_items(trash: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(trash.join("files"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn removing_with_data_and_file_moves_the_copy_and_the_data_to_the_trash() {
    let scratch = Scratch::new("trash");
    let path = scratch.copy(SOURCE, "book.epub");
    let original = fs::read(&path).unwrap();
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    append_paragraph_before_text(&book, first_text_chapter(&book));
    library.save_resume_point(key, false, resume("r.xhtml", "Gone.")).unwrap();
    let aliases = scratch.data().join("aliases");
    fs::create_dir_all(&aliases).unwrap();
    fs::write(aliases.join("ab".repeat(32)), crate::backup::hex_of(&book.identity().0)).unwrap();
    fs::write(aliases.join("cd".repeat(32)), "ef".repeat(32)).unwrap();
    drop(book);
    let removal = library.removal(key).unwrap();
    assert!(removal.data_bytes > 64);
    assert_eq!(removal.file_bytes, Some(fs::metadata(&copy).unwrap().len()));
    let trash_root = scratch.0.join("Trash");
    library
        .remove_with(key, true, true, &Trash::at(trash_root.clone()))
        .unwrap();
    assert!(library.shelf().is_empty());
    assert!(!copy.exists());
    assert_eq!(fs::read(&path).unwrap(), original, "the added file is never touched");
    assert!(!scratch.data().join("books").join(key_hex(key)).exists());
    assert!(aliases.join("cd".repeat(32)).exists(), "another book's alias stays");
    assert!(!aliases.join("ab".repeat(32)).exists());
    let copy_name = copy.file_name().unwrap().to_string_lossy().into_owned();
    let items = trash_items(&trash_root);
    let mut wanted = vec!["ab".repeat(32), copy_name.clone(), key_hex(key)];
    wanted.sort();
    assert_eq!(items, wanted);
    library.open_book(&trash_root.join("files").join(&copy_name)).unwrap();
    assert_eq!(library.known(key).unwrap().read_point, None);
}

#[test]
fn removing_a_record_left_outside_the_library_never_offers_or_trashes_that_file() {
    let scratch = Scratch::new("outside");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = key_of(&library.open_book(&path).unwrap());
    write_old_shelf(&scratch.data(), &[(key, &path)]);
    let library = scratch.library();
    assert_eq!(library.removal(key).unwrap().file_bytes, None);
    let trash_root = scratch.0.join("Trash");
    library
        .remove_with(key, false, true, &Trash::at(trash_root.clone()))
        .unwrap();
    assert!(path.exists());
    assert!(trash_items(&trash_root).is_empty());
}

#[test]
fn a_trash_that_fails_leaves_the_book_its_data_and_its_record() {
    let scratch = Scratch::new("trash-fails");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    let (anchor, _) = whole_range_in_second_line(&book, first_text_chapter(&book));
    library.add_mark(key, anchor, None).unwrap();
    library.save_resume_point(key, false, resume("r.xhtml", "Kept.")).unwrap();
    let book_dir = scratch.data().join("books").join(key_hex(key));
    assert!(book_dir.exists());
    let trash_root = scratch.0.join("Trash");
    fs::create_dir_all(trash_root.join("files")).unwrap();
    fs::write(trash_root.join("info"), b"not a folder").unwrap();
    let error = library
        .remove_with(key, true, true, &Trash::at(trash_root.clone()))
        .unwrap_err();
    assert!(error.user_message().contains("nothing was removed"), "{error:?}");
    assert!(copy.exists());
    assert!(book_dir.exists());
    assert_eq!(library.entry(key).unwrap().read_point.unwrap().passage, "Kept.");
    assert!(trash_items(&trash_root).is_empty());
}

#[test]
#[ignore = "timing on the largest real book"]
fn time_a_word_count_of_the_largest_real_book() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let reader = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|entry| Reader::open(File::open(entry.path()).ok()?, DEFAULT_LIMITS).ok())
        .max_by_key(|reader| reader.summary().linear_chapters)
        .unwrap();
    let mut times = Vec::new();
    let mut words = 0;
    for _ in 0..3 {
        let started = std::time::Instant::now();
        words = reader.word_count();
        times.push(started.elapsed().as_millis());
    }
    println!(
        "word count of {} chapters: {words} words in {times:?} ms",
        reader.summary().linear_chapters
    );
}

#[cfg(unix)]
#[test]
fn a_data_folder_that_will_not_move_brings_the_trashed_file_back() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new("trash-rollback");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    let (anchor, _) = whole_range_in_second_line(&book, first_text_chapter(&book));
    library.add_mark(key, anchor, None).unwrap();
    drop(book);
    let books = scratch.data().join("books");
    let trash_root = scratch.0.join("Trash");
    fs::set_permissions(&books, fs::Permissions::from_mode(0o555)).unwrap();
    let result = library.remove_with(key, true, true, &Trash::at(trash_root.clone()));
    fs::set_permissions(&books, fs::Permissions::from_mode(0o755)).unwrap();
    let error = result.unwrap_err();
    assert!(error.user_message().contains("Jadeslip's data for this book"), "{error:?}");
    assert!(copy.exists(), "the copy came back from the trash");
    assert!(path.exists());
    assert!(books.join(key_hex(key)).join(MARKS_FILE).exists());
    assert!(library.entry(key).is_some());
    assert!(trash_items(&trash_root).is_empty());
    assert!(fs::read_dir(trash_root.join("info")).unwrap().next().is_none());
}

#[test]
fn a_failed_removal_keeps_a_file_made_since_and_says_where_the_trashed_copy_is() {
    use crate::commit::seam::install;
    let scratch = Scratch::new("trash-rollback-taken");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let book = shelved(&library, &path);
    let copy = book.path().to_owned();
    let key = key_of(&book);
    drop(book);
    let trash_root = scratch.0.join("Trash");
    let made_since = copy.clone();
    install(move |step| {
        if step == Step::LibrarySynced {
            fs::write(&made_since, b"new work").unwrap();
            return Err(io::Error::other("the shelf could not be written"));
        }
        Ok(())
    });
    let result = library.remove_with(key, false, true, &Trash::at(trash_root.clone()));
    install(|_| Ok(()));
    let message = result.unwrap_err().user_message();
    assert!(
        fs::read(&copy).unwrap() == b"new work",
        "the file made since was replaced by the trashed copy"
    );
    let trashed = trash_items(&trash_root);
    assert_eq!(trashed.len(), 1);
    assert!(
        message.contains("Something new is in its place")
            && message.contains(&trash_root.join("files").display().to_string()),
        "{message}"
    );
    assert!(library.entry(key).is_some());
}

fn write_old_shelf(data: &Path, books: &[(BookKey, &Path)]) {
    let records: serde_json::Map<String, Value> = books
        .iter()
        .map(|(key, path)| {
            let title = File::open(path)
                .ok()
                .and_then(|file| Reader::open(file, DEFAULT_LIMITS).ok())
                .map_or_else(|| "Gone".to_owned(), |reader| reader.summary().title.clone());
            let record = serde_json::json!({
                "path": path,
                "title": title,
                "author": null,
                "inflated_spine_bytes": 100,
                "linear_chapters": 3,
                "own": false,
                "read_point": null,
                "write_point": null,
                "added_at": 1,
                "touched_at": 1,
            });
            (key_hex(*key), record)
        })
        .collect();
    fs::create_dir_all(data).unwrap();
    fs::write(
        data.join(LIBRARY_FILE),
        serde_json::to_vec(&serde_json::json!({"version": 1, "books": records})).unwrap(),
    )
    .unwrap();
}

struct OldShelf {
    scratch: Scratch,
    present: Vec<(BookKey, PathBuf)>,
    missing: (BookKey, PathBuf),
    originals: Vec<(PathBuf, Vec<u8>, std::time::SystemTime)>,
    shelf_bytes: Vec<u8>,
}

fn old_shelf_at(scratch: Scratch) -> OldShelf {
    let plain = scratch.copy(SOURCE, "Plain Book.epub");
    let covered = scratch.copy("cover-epub3", "covered.epub");
    let gone = scratch.copy("toc-ncx-only", "gone.epub");
    let aliased = scratch.copy("cover-epub2", "aliased.epub");
    let mine = scratch.0.join("mine.epub");
    let data = scratch.data();
    let library = scratch.library();
    let plain_key = key_of(&library.open_book(&plain).unwrap());
    let covered_book = library.open_book(&covered).unwrap();
    let covered_key = key_of(&covered_book);
    let (anchor, _) = whole_range_in_second_line(&covered_book, first_text_chapter(&covered_book));
    library.add_mark(covered_key, anchor, None).unwrap();
    drop(covered_book);
    let gone_key = key_of(&library.open_book(&gone).unwrap());
    let aliased_book = library.open_book(&aliased).unwrap();
    let aliased_key = key_of(&aliased_book);
    aliased_book
        .manage(
            ChapterOp::Add {
                after: None,
                title: "Added later".to_owned(),
            },
            None,
        )
        .unwrap();
    drop(aliased_book);
    let aliased_fingerprint = Reader::open(File::open(&aliased).unwrap(), DEFAULT_LIMITS)
        .unwrap()
        .summary()
        .fingerprint;
    assert_ne!(BookKey::of(&aliased_fingerprint), aliased_key, "the edit changed the fingerprint");
    assert_eq!(library.key_of_file(&aliased).unwrap(), aliased_key, "an alias keeps the key");
    let mine_key = key_of(&crate::manage::new_book(&mine, &data, &meta("Mine"), DEFAULT_LIMITS).unwrap());
    fs::remove_file(&gone).unwrap();
    for stash in fs::read_dir(data.join("books")).unwrap().flatten() {
        let _ = fs::remove_file(stash.path().join(STASH_FILE));
    }
    let canonical = |path: &Path| fs::canonicalize(path).unwrap();
    let present = vec![
        (plain_key, canonical(&plain)),
        (covered_key, canonical(&covered)),
        (aliased_key, canonical(&aliased)),
        (mine_key, canonical(&mine)),
    ];
    let missing = (gone_key, scratch.0.canonicalize().unwrap().join("gone.epub"));
    let mut records: Vec<(BookKey, &Path)> =
        present.iter().map(|(key, path)| (*key, path.as_path())).collect();
    records.push((missing.0, &missing.1));
    write_old_shelf(&data, &records);
    let file = data.join(LIBRARY_FILE);
    let mut root: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    root["books"][key_hex(covered_key)]["read_point"] =
        serde_json::to_value(resume("c.xhtml", "Kept through the move.")).unwrap();
    root["books"][key_hex(mine_key)]["own"] = Value::Bool(true);
    fs::write(&file, serde_json::to_vec(&root).unwrap()).unwrap();
    let originals = present
        .iter()
        .map(|(_, path)| {
            (
                path.clone(),
                fs::read(path).unwrap(),
                fs::metadata(path).unwrap().modified().unwrap(),
            )
        })
        .collect();
    OldShelf {
        shelf_bytes: fs::read(&file).unwrap(),
        scratch,
        present,
        missing,
        originals,
    }
}

impl OldShelf {
    fn originals_untouched(&self) {
        for (path, bytes, modified) in &self.originals {
            assert_eq!(&fs::read(path).unwrap(), bytes, "{} changed", path.display());
            assert_eq!(&fs::metadata(path).unwrap().modified().unwrap(), modified);
        }
        assert!(!self.missing.1.exists());
    }

    fn library_files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.scratch.data().join("library"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn assert_migrated(&self) {
        self.originals_untouched();
        let library = self.scratch.library();
        let shelf = library.shelf();
        assert_eq!(shelf.len(), 5);
        for ((key, original), (_, bytes, modified)) in self.present.iter().zip(&self.originals) {
            let entry = library.entry(*key).unwrap();
            assert!(library.is_managed(&entry.path), "{} was not moved in", original.display());
            assert_eq!(&fs::read(&entry.path).unwrap(), bytes);
            assert_eq!(&fs::metadata(&entry.path).unwrap().modified().unwrap(), modified);
            assert_eq!(library.key_of_file(&entry.path).unwrap(), *key);
        }
        assert_eq!(library.entry(self.missing.0).unwrap().path, self.missing.1);
        let files = self.library_files();
        assert_eq!(files.len(), 4, "one copy per book and no stage files: {files:?}");
        assert!(files.iter().all(|name| !name.starts_with('.')), "{files:?}");
        assert_eq!(
            fs::read(self.scratch.data().join("library.json.pre-managed")).unwrap(),
            self.shelf_bytes
        );
        let covered = library.entry(self.present[1].0).unwrap();
        assert_eq!(covered.read_point.unwrap().passage, "Kept through the move.");
        assert_eq!(library.marks(self.present[1].0).unwrap().len(), 1);
        assert!(library.entry(self.present[3].0).unwrap().own);
    }
}

#[test]
fn migration_copies_every_shelf_book_in_once_and_keeps_its_data() {
    let old = old_shelf_at(Scratch::new("migrate"));
    let library = old.scratch.library();
    let mut seen = Vec::new();
    let report = library.migrate(|state| seen.push(state.done));
    assert_eq!(report.total, 4);
    assert_eq!(report.copied, 4);
    assert_eq!(report.left, 0);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(seen, [0, 1, 2, 3, 4]);
    old.assert_migrated();
    let names = old.library_files();
    assert!(names.contains(&format!("Toc Nav Only - {}.epub", &key_hex(old.present[0].0)[..8])), "{names:?}");

    let shelf_file = old.scratch.data().join(LIBRARY_FILE);
    let settled = fs::read(&shelf_file).unwrap();
    let again = old.scratch.library().migrate(|_| {});
    assert_eq!(again, Migration::default(), "a missing book is not counted again");
    assert_eq!(fs::read(&shelf_file).unwrap(), settled, "a second run changes nothing");
    old.assert_migrated();

    let library = old.scratch.library();
    let aliased = library.entry(old.present[2].0).unwrap();
    assert_eq!(key_of(&library.open_book(&aliased.path).unwrap()), old.present[2].0, "the aliased copy opens as the same book");
}

#[test]
fn a_shelf_without_old_records_makes_no_backup_and_no_folder() {
    let scratch = Scratch::new("nothing");
    let library = scratch.library();
    assert_eq!(library.migrate(|_| {}), Migration::default());
    assert!(!scratch.data().join("library.json.pre-managed").exists());
    let path = scratch.copy(SOURCE, "book.epub");
    library.add_to_shelf(&path).unwrap();
    assert_eq!(library.migrate(|_| {}).total, 0);
    assert!(!scratch.data().join("library.json.pre-managed").exists());
}

#[test]
fn a_found_file_heals_a_record_the_migration_left_behind() {
    let old = old_shelf_at(Scratch::new("heal"));
    old.scratch.library().migrate(|_| {});
    fs::copy(fixture("toc-ncx-only"), &old.missing.1).unwrap();
    let library = old.scratch.library();
    let add = library.add_to_shelf(&old.missing.1).unwrap();
    assert_eq!(add.key, old.missing.0);
    assert_eq!(add.outcome, AddOutcome::Added);
    let entry = library.entry(old.missing.0).unwrap();
    assert!(library.is_managed(&entry.path));
    assert_eq!(library.shelf().len(), 5);
}

#[test]
fn two_records_on_one_file_move_only_the_book_the_file_holds() {
    let scratch = Scratch::new("shared-path");
    let path = scratch.copy(SOURCE, "book.epub");
    let other = scratch.copy("toc-ncx-only", "other.epub");
    let library = scratch.library();
    let real_key = library.key_of_file(&path).unwrap();
    let stale_key = library.key_of_file(&other).unwrap();
    let path = fs::canonicalize(&path).unwrap();
    write_old_shelf(&scratch.data(), &[(real_key, &path), (stale_key, &path)]);
    let report = scratch.library().migrate(|_| {});
    assert_eq!((report.total, report.copied), (1, 1));
    let library = scratch.library();
    assert!(library.is_managed(&library.entry(real_key).unwrap().path));
    assert_eq!(library.entry(stale_key).unwrap().path, path);
    assert_eq!(fs::read_dir(library.library_dir()).unwrap().count(), 1);
}

#[test]
fn books_with_one_title_get_separate_copies() {
    let scratch = Scratch::new("same-title");
    let one = scratch.copy(SOURCE, "one.epub");
    let library = scratch.library();
    let first = library.add_to_shelf(&one).unwrap();
    let edited = scratch.copy(SOURCE, "two.epub");
    let book = library.open_book(&edited).unwrap();
    append_paragraph_before_text(&book, first_text_chapter(&book));
    book.manage(
        ChapterOp::Add {
            after: None,
            title: "More".to_owned(),
        },
        None,
    )
    .unwrap();
    drop(book);
    fs::remove_dir_all(scratch.data().join("aliases")).unwrap();
    let second = library.add_to_shelf(&edited).unwrap();
    assert_ne!(first.key, second.key);
    let one_path = library.entry(first.key).unwrap().path;
    let two_path = library.entry(second.key).unwrap().path;
    assert_ne!(one_path, two_path);
    assert_eq!(fs::read(&two_path).unwrap(), fs::read(&edited).unwrap());
}

#[test]
fn a_leftover_file_under_a_copy_name_is_never_overwritten() {
    let scratch = Scratch::new("taken-name");
    let path = scratch.copy(SOURCE, "book.epub");
    let library = scratch.library();
    let key = library.key_of_file(&path).unwrap();
    let dir = library.library_dir();
    fs::create_dir_all(&dir).unwrap();
    let taken = dir.join(format!("Toc Nav Only - {}.epub", &key_hex(key)[..8]));
    fs::write(&taken, b"somebody else's bytes").unwrap();
    let add = library.add_to_shelf(&path).unwrap();
    assert_eq!(fs::read(&taken).unwrap(), b"somebody else's bytes");
    let copy = library.entry(add.key).unwrap().path;
    assert_eq!(
        copy.file_name().unwrap().to_string_lossy(),
        format!("Toc Nav Only - {} (2).epub", &key_hex(key)[..8])
    );
}

#[test]
fn two_writes_of_one_data_file_at_once_never_fail_or_tear() {
    use std::sync::atomic::AtomicBool;
    let scratch = Scratch::new("concurrent-write");
    let library = scratch.library();
    let key = BookKey(0x5678);
    library
        .set_spelling_choice(key, &SpellChoice::Auto)
        .unwrap();
    let path = library.spelling_path(key);
    let stop = Arc::new(AtomicBool::new(false));
    let writers: Vec<_> = [SpellChoice::Auto, SpellChoice::Installed("x".repeat(4000))]
        .into_iter()
        .map(|choice| {
            let library = Arc::clone(&library);
            std::thread::spawn(move || {
                (0..500)
                    .filter(|_| library.set_spelling_choice(key, &choice).is_err())
                    .count()
            })
        })
        .collect();
    let reader = {
        let stop = Arc::clone(&stop);
        let path = path.clone();
        std::thread::spawn(move || {
            let mut torn = 0;
            while !stop.load(Ordering::Relaxed) {
                if let Ok(bytes) = fs::read(&path)
                    && serde_json::from_slice::<Value>(&bytes).is_err()
                {
                    torn += 1;
                }
            }
            torn
        })
    };
    let failed: usize = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .sum();
    stop.store(true, Ordering::Relaxed);
    let torn = reader.join().unwrap();
    let last_parses = serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).is_ok();
    assert!(
        failed == 0 && torn == 0 && last_parses,
        "failed writes: {failed}, torn reads: {torn}, final file parses: {last_parses}"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_at_the_temp_name_does_not_redirect_a_data_write_outside() {
    let scratch = Scratch::new("temp-symlink");
    let outside = scratch.0.join("outside.txt");
    fs::write(&outside, b"precious").unwrap();
    let library = scratch.library();
    let key = BookKey(0x1234);
    let path = library.spelling_path(key);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let temp = path.with_file_name(format!("{SPELLING_FILE}.{}.tmp", std::process::id()));
    std::os::unix::fs::symlink(&outside, &temp).unwrap();
    library
        .set_spelling_choice(key, &SpellChoice::English)
        .unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "precious");
    assert_eq!(library.spelling_choice(key), SpellChoice::English);
}

#[cfg(unix)]
#[test]
fn a_data_folder_reached_through_a_symlink_counts_its_books_and_their_copies() {
    let scratch = Scratch::new("linked-data");
    let link = scratch.0.join("linked-data");
    std::os::unix::fs::symlink(scratch.data(), &link).unwrap();
    let path = scratch.copy(SOURCE, "book.epub");
    let library = Library::open(LibraryConfig { data_dir: link }).unwrap();
    let key = library.add_to_shelf(&path).unwrap().key;
    let direct = crate::storage::usage(&scratch.data()).total;
    assert!(direct > 0);
    assert_eq!(crate::storage::usage(library.data_dir()).total, direct);
    assert!(
        library
            .entry(key)
            .unwrap()
            .path
            .starts_with(library.library_dir()),
        "the storage page counts the shelved copy as the library's"
    );
}

const KILL_DATA: &str = "JADESLIP_KILL_DATA";
const KILL_AT: &str = "JADESLIP_KILL_AT";
const KILLED: i32 = 86;

#[test]
#[ignore = "run by migration_resumes_after_a_kill_at_every_step"]
fn migration_child() {
    let (Some(data), Some(at)) = (std::env::var_os(KILL_DATA), std::env::var_os(KILL_AT)) else {
        return;
    };
    let at: usize = at.to_string_lossy().parse().unwrap();
    let mut hits = 0;
    crate::commit::seam::install(move |_| {
        hits += 1;
        if hits == at {
            std::process::exit(KILLED);
        }
        Ok(())
    });
    Library::open(LibraryConfig {
        data_dir: PathBuf::from(data),
    })
    .unwrap()
    .migrate(|_| {});
}

#[test]
fn migration_resumes_after_a_kill_at_every_step() {
    use crate::commit::seam::install;
    use std::cell::RefCell;
    use std::rc::Rc;
    let counted = old_shelf_at(Scratch::new("kill-count"));
    let steps = Rc::new(RefCell::new(Vec::new()));
    let log = steps.clone();
    install(move |step| {
        log.borrow_mut().push(step);
        Ok(())
    });
    counted.scratch.library().migrate(|_| {});
    install(|_| Ok(()));
    counted.assert_migrated();
    let steps = steps.borrow().clone();
    assert!(steps.len() > 20, "{steps:?}");
    for (at, step) in steps.iter().enumerate().map(|(ix, step)| (ix + 1, step)) {
        let old = old_shelf_at(Scratch::new("kill"));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "library::tests::migration_child", "--ignored", "--quiet"])
            .env(KILL_DATA, old.scratch.data())
            .env(KILL_AT, at.to_string())
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(KILLED), "the child was not killed at {step:?}");
        old.originals_untouched();
        let halfway = old.scratch.library();
        assert_eq!(halfway.shelf().len(), 5, "killed at {step:?}, the shelf still reads");
        for (key, _) in &old.present {
            let entry = halfway.entry(*key).unwrap();
            if halfway.is_managed(&entry.path) {
                assert_eq!(halfway.key_of_file(&entry.path).unwrap(), *key);
            }
        }
        old.scratch.library().migrate(|_| {});
        old.assert_migrated();
    }
    println!("killed and resumed at {} steps: {steps:?}", steps.len());
}
