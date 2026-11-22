use jadeslip_core::{
    BlockChange, BlockEdits, BlockIx, Book, ChapterIx, DEFAULT_LIMITS, Extracted, Reader,
};
use jadeslip_e2e::{
    App, KEY_BACKSPACE, KEY_ESCAPE, generated_book, largest_real_book, scratch_dir,
};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const INSTALL: &str = r"(async()=>{
  const main = await import('/main.js');
  const host = document.getElementById('book-host');
  const element = (id) => document.getElementById(id);
  const root = () => host.shadowRoot.querySelector('.ss-body');
  window.__errors = [];
  window.addEventListener('error', (event) => window.__errors.push(event.message));
  window.addEventListener('unhandledrejection', (event) => window.__errors.push(String(event.reason?.message ?? event.reason)));
  window.__t = {
    root,
    mount: async (ix) => { await main.goTo(ix); return true; },
    status: () => ({
      editable: root()?.getAttribute('contenteditable') ?? null,
      read: element('mode-read').classList.contains('on'),
      unsaved: !element('unsaved').hidden,
      fail: element('failbar').hidden ? '' : element('fail-text').textContent,
      toast: element('toast').hidden ? '' : element('toast').textContent,
      offer: element('draft-offer').hidden ? '' : element('draft-offer-text').textContent,
      recover: !element('draft-recover').hidden,
      shown: main.shownChapter(),
    }),
    caretInLongText: (fraction) => {
      const blocks = [...root().querySelectorAll('[data-ss-b]')];
      const block = blocks.sort((a, b) => b.textContent.length - a.textContent.length)[0] ?? root();
      const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
      const nodes = [];
      while (walker.nextNode()) if (walker.currentNode.data.trim().length > 20) nodes.push(walker.currentNode);
      const node = nodes[Math.floor(nodes.length * fraction)];
      const offset = node.data.indexOf(' ', Math.floor(node.data.length / 2));
      const at = offset > 0 ? offset : 1;
      document.getSelection().setBaseAndExtent(node, at, node, at);
      return true;
    },
    press: (id) => { element(id).click(); return true; },
  };
  return true;
})()";

const IN_READ: &str =
    "(()=>{const s=window.__t.status();return s.editable===null&&s.read&&!s.unsaved})()";
const IN_EDIT: &str = "window.__t.status().editable==='true'";
const DIRTY: &str = "window.__t.status().unsaved";
const CLEAN: &str = "!window.__t.status().unsaved";
const OFFERED: &str = "window.__t.status().offer!==''";
const TYPED: &str = "qzjx";

struct Copy {
    dir: PathBuf,
    path: PathBuf,
    data: PathBuf,
}

impl Copy {
    fn of(source: &Path, label: &str) -> Copy {
        let dir = scratch_dir().join(format!("drafts-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.epub");
        fs::write(&path, fs::read(source).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        Copy {
            data: dir.join("data"),
            dir,
            path,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(&self.path).unwrap()
    }

    fn open(&self) -> App {
        let mut app = App::launch_with_data(&self.path, &self.data);
        app.wait_first_text();
        app.assert_on_headless_output();
        app.js(INSTALL);
        app
    }

    fn drafts(&self) -> Vec<PathBuf> {
        let books = self.data.join("books");
        fs::read_dir(books)
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|entry| {
                fs::read_dir(entry.path().join("drafts"))
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect()
    }

    fn wait_for_draft(&self) -> Duration {
        let started = Instant::now();
        while self.drafts().is_empty() {
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "no draft was written"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        started.elapsed()
    }

    fn wait_for_no_draft(&self) {
        let started = Instant::now();
        while !self.drafts().is_empty() {
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "the draft was not removed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn until(app: &mut App, what: &str, script: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if app.js(script).as_bool() == Some(true) {
            return;
        }
        if Instant::now() > deadline {
            let errors = app.js("window.__errors");
            let status = app.js("window.__t.status()");
            panic!("timed out waiting: {what}, page errors {errors}, status {status}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn status(app: &mut App) -> Value {
    app.js("window.__t.status()")
}

fn draft_writes(app: &mut App) -> usize {
    app.phases()
        .iter()
        .filter(|(name, _)| name == "draft_write")
        .count()
}

fn text_chapter(path: &Path, skip: usize) -> u32 {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .filter(|row| {
            matches!(reader.extract(row.ix), Ok(Extracted::Document(bytes)) if bytes.len() > 2000)
        })
        .nth(skip)
        .expect("a text chapter")
        .ix
        .0
}

fn type_in(app: &mut App, ix: u32) {
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(app, "Edit after E", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text(TYPED);
    until(app, "dirty after typing", DIRTY);
}

fn crash_after_typing(copy: &Copy, ix: u32) -> Duration {
    let original = copy.bytes();
    let mut app = copy.open();
    type_in(&mut app, ix);
    let typed_at = Instant::now();
    copy.wait_for_draft();
    let waited = typed_at.elapsed();
    assert_eq!(copy.bytes(), original, "the book file is untouched");
    assert!(app.js("window.__t.status().unsaved").as_bool() == Some(true));
    app.close();
    assert_eq!(copy.drafts().len(), 1, "the draft outlives the crash");
    waited
}

#[test]
#[ignore = "GUI and real books: SS_E2E_TEST=drafts tests/e2e/run.sh"]
fn a_crash_keeps_the_typed_edits_and_recovering_saves_what_a_normal_save_saves() {
    let source = largest_real_book();
    let ix = text_chapter(&source, 40);

    let normal = Copy::of(&source, "normal");
    let mut app = normal.open();
    type_in(&mut app, ix);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read after Esc", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    app.close();
    assert_ne!(normal.bytes(), fs::read(&source).unwrap());

    let crashed = Copy::of(&source, "crashed");
    let waited = crash_after_typing(&crashed, ix);
    println!("draft on disk {} ms after the last key", waited.as_millis());
    assert!(
        waited >= Duration::from_millis(1500),
        "the draft waits for a pause in typing"
    );

    let mut app = crashed.open();
    until(&mut app, "the recovery offer", OFFERED);
    assert_eq!(status(&mut app)["recover"], json!(true));
    app.js("window.__t.press('draft-recover')");
    until(&mut app, "Edit with the recovered edits", IN_EDIT);
    until(&mut app, "the recovered edits are unsaved", DIRTY);
    let shown = status(&mut app);
    assert_eq!(shown["shown"], json!(ix));
    assert_eq!(shown["offer"], json!(""));
    assert_eq!(
        app.js(&format!(
            "window.__t.root().textContent.includes({})",
            json!(TYPED)
        )),
        json!(true)
    );
    app.key(KEY_ESCAPE);
    until(&mut app, "Read after saving", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    crashed.wait_for_no_draft();
    app.close();
    assert!(
        crashed.bytes() == normal.bytes(),
        "the recovered save writes the same book as a normal save"
    );

    let mut app = crashed.open();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        status(&mut app)["offer"],
        json!(""),
        "nothing left to offer"
    );
    app.close();
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=drafts tests/e2e/run.sh"]
fn discarding_the_offer_removes_the_draft_and_leaves_the_book() {
    let copy = Copy::of(&generated_book("large-h1-p"), "discard");
    let original = copy.bytes();
    let ix = text_chapter(&copy.path, 3);
    crash_after_typing(&copy, ix);

    let mut app = copy.open();
    until(&mut app, "the recovery offer", OFFERED);
    app.key(u32::from('e'));
    until(
        &mut app,
        "Edit is refused while the offer is open",
        "window.__t.status().toast.includes('Recover or discard')",
    );
    assert_eq!(status(&mut app)["editable"], Value::Null);
    app.js("window.__t.press('draft-discard')");
    until(
        &mut app,
        "the offer is gone",
        "window.__t.status().offer===''",
    );
    assert!(copy.drafts().is_empty(), "discard removes the draft");
    app.key(u32::from('e'));
    until(&mut app, "Edit after discarding", IN_EDIT);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    app.close();
    assert_eq!(copy.bytes(), original, "discard leaves the book as it was");

    let mut app = copy.open();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(status(&mut app)["offer"], json!(""));
    app.close();
}

fn edit_elsewhere(path: &Path, data: &Path, ix: u32) {
    let book = Book::open_by_fingerprint(path, data, DEFAULT_LIMITS).unwrap();
    let reader = book.read();
    let rendered = reader.render(ChapterIx(ix)).unwrap();
    let block = jadeslip_core::proof::block_xml_of(&rendered.xhtml).unwrap();
    let edited = block[0].replacen("</", "vvvv</", 1);
    book.save_chapter(
        ChapterIx(ix),
        &BlockEdits {
            base: rendered.rev,
            changes: vec![BlockChange::Replace {
                of: BlockIx(0),
                xml: edited,
            }],
        },
    )
    .unwrap();
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=drafts tests/e2e/run.sh"]
fn two_windows_on_one_book_each_keep_their_own_draft_through_a_crash() {
    let copy = Copy::of(&generated_book("large-h1-p"), "two-windows");
    let original = copy.bytes();
    let first_ix = text_chapter(&copy.path, 3);
    let second_ix = text_chapter(&copy.path, 5);
    assert_ne!(first_ix, second_ix);

    let mut first = copy.open();
    let mut second = copy.open();
    type_in(&mut first, first_ix);
    type_in(&mut second, second_ix);
    let started = Instant::now();
    while copy.drafts().len() < 2 {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "both windows write a draft: {:?}",
            copy.drafts()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    first.close();
    second.close();
    assert_eq!(copy.bytes(), original, "the book file is untouched");
    let mut chapters: Vec<u64> = copy
        .drafts()
        .iter()
        .map(|path| {
            let stored: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            stored["chapter"].as_u64().unwrap()
        })
        .collect();
    chapters.sort_unstable();
    let mut expected = vec![u64::from(first_ix), u64::from(second_ix)];
    expected.sort_unstable();
    assert_eq!(chapters, expected, "each window's edits survive the crash");

    let mut app = copy.open();
    until(&mut app, "the first offer", OFFERED);
    app.js("window.__t.press('draft-discard')");
    let started = Instant::now();
    while copy.drafts().len() != 1 {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "discard removes only the offered draft: {:?}",
            copy.drafts()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    until(
        &mut app,
        "the second offer after discarding the first",
        OFFERED,
    );
    assert_eq!(status(&mut app)["recover"], json!(true));
    let left: Value = serde_json::from_slice(&fs::read(&copy.drafts()[0]).unwrap()).unwrap();
    app.js("window.__t.press('draft-recover')");
    until(&mut app, "the recovered edits", DIRTY);
    assert_eq!(status(&mut app)["shown"], left["chapter"]);
    app.close();
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=drafts tests/e2e/run.sh"]
fn a_draft_for_a_book_changed_since_is_refused() {
    let copy = Copy::of(&generated_book("large-h1-p"), "changed");
    let ix = text_chapter(&copy.path, 3);
    crash_after_typing(&copy, ix);
    edit_elsewhere(&copy.path, &copy.dir.join("elsewhere"), ix + 1);
    let changed = copy.bytes();

    let mut app = copy.open();
    until(&mut app, "the offer", OFFERED);
    let shown = status(&mut app);
    assert!(
        shown["offer"].as_str().unwrap().contains("changed since"),
        "the offer says the book changed: {shown}"
    );
    assert_eq!(shown["recover"], json!(false), "no recover button");
    app.js("window.__t.press('draft-recover')");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(status(&mut app)["editable"], Value::Null);
    assert_eq!(copy.drafts().len(), 1, "the draft is kept until discarded");
    app.js("window.__t.press('draft-discard')");
    until(
        &mut app,
        "the offer is gone",
        "window.__t.status().offer===''",
    );
    assert!(copy.drafts().is_empty());
    app.close();
    assert_eq!(copy.bytes(), changed, "the refused draft changed nothing");
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=drafts tests/e2e/run.sh"]
fn a_normal_save_removes_the_draft() {
    let copy = Copy::of(&generated_book("large-h1-p"), "save");
    let ix = text_chapter(&copy.path, 3);
    let mut app = copy.open();
    type_in(&mut app, ix);
    copy.wait_for_draft();
    app.key(KEY_ESCAPE);
    until(&mut app, "Read after Esc", IN_READ);
    copy.wait_for_no_draft();
    app.close();
    let mut app = copy.open();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(status(&mut app)["offer"], json!(""));
    app.close();
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=drafts tests/e2e/run.sh"]
fn no_draft_is_written_in_read_or_without_changes() {
    let copy = Copy::of(&generated_book("large-h1-p"), "none");
    let ix = text_chapter(&copy.path, 3);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    std::thread::sleep(Duration::from_millis(3000));
    assert!(copy.drafts().is_empty(), "no draft in Read");

    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    std::thread::sleep(Duration::from_millis(3000));
    assert!(copy.drafts().is_empty(), "no draft in Edit without typing");

    app.js("window.__t.caretInLongText(0.5)");
    app.key(u32::from('q'));
    until(&mut app, "dirty", DIRTY);
    app.key(KEY_BACKSPACE);
    until(&mut app, "clean after Backspace", CLEAN);
    std::thread::sleep(Duration::from_millis(3000));
    assert!(copy.drafts().is_empty(), "no draft once the edit is undone");
    assert_eq!(draft_writes(&mut app), 0);

    app.key(u32::from('q'));
    until(&mut app, "dirty", DIRTY);
    copy.wait_for_draft();
    app.key(KEY_BACKSPACE);
    until(&mut app, "clean after Backspace", CLEAN);
    copy.wait_for_no_draft();
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    app.close();
}

const TIMER: &str = r"(()=>{
  window.__keys = [];
  const twoFrames = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'q') return;
    const start = performance.now();
    const at = performance.timeOrigin + start;
    twoFrames().then(() => window.__keys.push({ at, painted: performance.now() - start }));
  }, true);
  return true;
})()";

fn percentile(values: &mut [f64], share: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * share).round() as usize]
}

#[test]
#[ignore = "GUI and real books: SS_E2E_TEST=drafts tests/e2e/run.sh"]
fn typing_stays_as_fast_while_drafts_are_written_on_the_largest_real_book() {
    let source = largest_real_book();
    let copy = Copy::of(&source, "latency");
    let ix = text_chapter(&copy.path, 100);
    let mut app = copy.open();
    app.js(TIMER);
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    for _ in 0..12 {
        for _ in 0..10 {
            app.key(u32::from('q'));
            std::thread::sleep(Duration::from_millis(60));
        }
        let written = draft_writes(&mut app);
        std::thread::sleep(Duration::from_millis(1900));
        let deadline = Instant::now() + Duration::from_secs(4);
        while draft_writes(&mut app) == written && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..8 {
            app.key(u32::from('q'));
            std::thread::sleep(Duration::from_millis(8));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    std::thread::sleep(Duration::from_millis(500));
    let keys = app.js("window.__keys");
    let writes: Vec<f64> = app
        .phases()
        .iter()
        .filter(|(name, _)| name == "draft_write")
        .map(|(_, at)| *at)
        .collect();
    let (mut near, mut far) = (Vec::new(), Vec::new());
    for key in keys.as_array().unwrap() {
        let at = key["at"].as_f64().unwrap();
        let painted = key["painted"].as_f64().unwrap();
        if writes.iter().any(|write| (at - write).abs() < 60.0) {
            near.push(painted);
        } else {
            far.push(painted);
        }
    }
    println!(
        "draft writes {} | keys near a write {} median {:.1} p95 {:.1} max {:.1} ms | other keys {} median {:.1} p95 {:.1} max {:.1} ms",
        writes.len(),
        near.len(),
        percentile(&mut near, 0.5),
        percentile(&mut near, 0.95),
        percentile(&mut near, 1.0),
        far.len(),
        percentile(&mut far, 0.5),
        percentile(&mut far, 0.95),
        percentile(&mut far, 1.0),
    );
    assert!(writes.len() >= 10, "a draft was written at most pauses");
    assert!(near.len() >= 10, "keys landed during draft writes");
    assert!(
        percentile(&mut near, 0.5) <= percentile(&mut far, 0.5) * 1.5 + 4.0,
        "keys during a draft write paint about as fast as others"
    );
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    app.close();
}
