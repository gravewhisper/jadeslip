use serde_json::{Value, json};
use jadeslip_core::{
    BlockChange, BlockEdits, BlockIx, Book, BookKey, ChapterIx, DEFAULT_LIMITS, Extracted, Reader,
};
use jadeslip_e2e::{
    App, data_files_besides_covers, place_name, KEY_BACKSPACE, KEY_ENTER, KEY_ESCAPE, KEY_LEFT, KEY_RIGHT, MASK_CTRL, MASK_SHIFT,
    generated_book, largest_real_book, real_books, scratch_dir,
};
use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const INSTALL: &str = r"(async()=>{
  const main = await import('/main.js');
  const ipc = await import('/ipc.js');
  const host = document.getElementById('book-host');
  const element = (id) => document.getElementById(id);
  const scroller = element('scroller');
  const root = () => host.shadowRoot.querySelector('.ss-body');
  const shadow = () => host.shadowRoot;
  const firstText = (from) => {
    const walker = document.createTreeWalker(from, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) if (walker.currentNode.data.trim().length > 20) return walker.currentNode;
    return null;
  };
  window.__errors = [];
  window.addEventListener('error', (event) => window.__errors.push(event.message));
  window.addEventListener('unhandledrejection', (event) => window.__errors.push(String(event.reason?.message ?? event.reason)));
  window.__t = {
    main, ipc, root,
    mount: async (ix) => { await main.goTo(ix); return true; },
    status: () => ({
      editable: root()?.getAttribute('contenteditable') ?? null,
      read: element('mode-read').classList.contains('on'),
      edit: element('mode-edit').classList.contains('on'),
      toolbar: !element('toolbar').hidden,
      unsaved: !element('unsaved').hidden,
      dialog: !element('dialog').hidden,
      fail: element('failbar').hidden ? '' : element('fail-text').textContent,
      reload: !element('fail-reload').hidden,
      toast: element('toast').hidden ? '' : element('toast').textContent,
      scroll: scroller.scrollTop,
      where: element('where-title').title,
    }),
    shape: () => {
      const blocks = [...root().querySelectorAll('[data-ss-b]')];
      return { blocks: blocks.length, tags: blocks.slice(0, 4).map((b) => b.localName), text: root().textContent.length, breaks: root().querySelectorAll('br').length };
    },
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
      return { block: Number(block.getAttribute('data-ss-b')), offset: at };
    },
    pointOfText: (ratio) => {
      const node = firstText(root());
      node.parentElement.scrollIntoView({ block: 'center' });
      const at = Math.floor(node.data.length * ratio);
      const range = document.createRange();
      range.setStart(node, at);
      range.setEnd(node, at + 1);
      const box = range.getBoundingClientRect();
      return { x: box.x + box.width / 2, y: box.y + box.height / 2, at, length: node.data.length };
    },
    caret: () => {
      const [range] = document.getSelection().getComposedRanges({ shadowRoots: [shadow()] });
      if (!range) return null;
      return { node: range.startContainer.nodeName, offset: range.startOffset, collapsed: range.collapsed, inside: shadow().contains(range.startContainer) };
    },
    anchorY: () => {
      const [range] = document.getSelection().getComposedRanges({ shadowRoots: [shadow()] });
      const probe = document.createRange();
      probe.setStart(range.startContainer, range.startOffset);
      const box = probe.getClientRects()[0] ?? (range.startContainer.nodeType === 1 ? range.startContainer : range.startContainer.parentElement).getBoundingClientRect();
      return box.top;
    },
    textY: () => host.getBoundingClientRect().top,
    scrollTo: (ratio) => { scroller.scrollTop = scroller.scrollHeight * ratio; return scroller.scrollTop; },
    buttonAt: (id) => {
      const box = element(id).getBoundingClientRect();
      return { x: box.x + box.width / 2, y: box.y + box.height / 2, disabled: element(id).disabled, hidden: box.width === 0 };
    },
    paste: (flavours) => {
      const data = new DataTransfer();
      for (const [type, value] of Object.entries(flavours)) data.setData(type, value);
      const target = root();
      const event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true, composed: true });
      target.dispatchEvent(event);
      return event.defaultPrevented;
    },
    selectWord: () => {
      const blocks = [...root().querySelectorAll('[data-ss-b]')];
      const block = blocks.sort((a, b) => b.textContent.length - a.textContent.length)[0] ?? root();
      const node = firstText(block);
      const end = node.data.indexOf(' ', 3);
      const word = document.createRange();
      word.setStart(node, 0);
      word.setEnd(node, end > 0 ? end : 3);
      const scroller = document.getElementById('scroller');
      const view = scroller.getBoundingClientRect();
      scroller.scrollTop += word.getBoundingClientRect().top - view.top - view.height / 2;
      document.getSelection().setBaseAndExtent(node, 0, node, end > 0 ? end : 3);
      return true;
    },
    pwned: () => window.__pwned ?? 0,
  };
  return true;
})()";

struct Copy {
    dir: PathBuf,
    path: PathBuf,
}

impl Copy {
    fn of(source: &Path, label: &str) -> Copy {
        let dir = scratch_dir().join(format!("edit-{label}-{}", std::process::id()));
        let _ = make_writable(&dir);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.epub");
        fs::write(&path, fs::read(source).unwrap()).unwrap();
        Copy { dir, path }
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(&self.path).unwrap()
    }

    fn open(&self) -> App {
        let mut app = App::launch(Some(&self.path), &[]);
        app.wait_first_text();
        app.assert_on_headless_output();
        app.js(INSTALL);
        app
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = make_writable(&self.dir);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn make_writable(dir: &Path) -> std::io::Result<()> {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755))?;
    fs::set_permissions(dir.join("book.epub"), fs::Permissions::from_mode(0o644))
}

fn chapter_bytes(path: &Path, ix: u32) -> Vec<u8> {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    match reader.extract(ChapterIx(ix)).unwrap() {
        Extracted::Document(bytes) => bytes,
        Extracted::Image { .. } => panic!("chapter {ix} is an image"),
    }
}

fn changed_chapters(before: &Path, after: &Path) -> Vec<u32> {
    let old = Reader::open(File::open(before).unwrap(), DEFAULT_LIMITS).unwrap();
    let new = Reader::open(File::open(after).unwrap(), DEFAULT_LIMITS).unwrap();
    assert_eq!(old.rows().len(), new.rows().len());
    (0..old.rows().len() as u32)
        .filter(|ix| {
            let bytes = |reader: &Reader| match reader.extract(ChapterIx(*ix)) {
                Ok(Extracted::Document(bytes)) => bytes,
                Ok(Extracted::Image { bytes, .. }) => bytes,
                Err(_) => Vec::new(),
            };
            bytes(&old) != bytes(&new)
        })
        .collect()
}

fn changed_region<'a>(old: &'a [u8], new: &'a [u8]) -> (&'a [u8], &'a [u8]) {
    let shared = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[shared..]
        .iter()
        .rev()
        .zip(new[shared..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (
        &old[shared..old.len() - tail],
        &new[shared..new.len() - tail],
    )
}

fn diff_of(before: &Path, after: &Path, ix: u32) -> (Vec<u8>, Vec<u8>) {
    let (old, new) = (chapter_bytes(before, ix), chapter_bytes(after, ix));
    let (was, now) = changed_region(&old, &new);
    (was.to_vec(), now.to_vec())
}

fn text_chapter(path: &Path, wanted: impl Fn(&[u8]) -> bool, skip: u32) -> u32 {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    let mut left = skip;
    for ix in 0..reader.rows().len() as u32 {
        if reader.rows()[ix as usize].is_nav {
            continue;
        }
        let Ok(Extracted::Document(bytes)) = reader.extract(ChapterIx(ix)) else {
            continue;
        };
        if wanted(&bytes) {
            if left == 0 {
                return ix;
            }
            left -= 1;
        }
    }
    panic!("no chapter of the wanted shape");
}

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn div_br_chapter(bytes: &[u8]) -> bool {
    has(bytes, "<br") && !has(bytes, "<p")
}

fn status(app: &mut App) -> Value {
    app.js("window.__t.status()")
}

fn until(app: &mut App, what: &str, script: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let value = app.js(script);
        if value.as_bool() == Some(true) {
            return value;
        }
        if Instant::now() > deadline {
            let errors = app.js("window.__errors");
            panic!(
                "timed out waiting: {what}, page errors {errors}, status {}",
                status(app)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

const IN_READ: &str =
    "(()=>{const s=window.__t.status();return s.editable===null&&s.read&&!s.unsaved})()";
const IN_EDIT: &str = "window.__t.status().editable==='true'";
const DIRTY: &str = "window.__t.status().unsaved";
const CLEAN: &str = "!window.__t.status().unsaved";

fn frames(app: &mut App) {
    app.js("new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(()=>done(true))))");
}

fn save_calls(app: &mut App) -> usize {
    app.phases()
        .iter()
        .filter(|(name, _)| name == "save_chapter")
        .count()
}

fn click_button(app: &mut App, id: &str) {
    let at = app.js(&format!("window.__t.buttonAt('{id}')"));
    assert_eq!(at["disabled"], json!(false), "{id} must be enabled");
    assert_eq!(at["hidden"], json!(false), "{id} must be visible");
    app.click(at["x"].as_f64().unwrap(), at["y"].as_f64().unwrap());
}

fn parses_as_xml(bytes: &[u8]) -> bool {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    std::str::from_utf8(bytes)
        .is_ok_and(|text| roxmltree::Document::parse_with_options(text, options).is_ok())
}

fn ebook_meta_opens(path: &Path) -> bool {
    Command::new("ebook-meta")
        .arg(path)
        .output()
        .is_ok_and(|done| done.status.success())
}

fn round_trip_holds(path: &Path, ix: u32, marker: &str) {
    assert!(
        parses_as_xml(&chapter_bytes(path, ix)),
        "saved chapter parses"
    );
    assert!(ebook_meta_opens(path), "ebook-meta opens the saved book");
    let mut app = App::launch(Some(path), &[]);
    app.wait_first_text();
    app.js(INSTALL);
    app.js(&format!("window.__t.mount({ix})"));
    let seen = app.js(&format!(
        "window.__t.root().textContent.includes({})",
        json!(marker)
    ));
    assert_eq!(seen, json!(true), "the saved text shows after a re-open");
    app.close();
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn e_type_esc_changes_only_the_typed_bytes_and_keeps_the_place() {
    let source = largest_real_book();
    let copy = Copy::of(&source, "typo");
    let before = copy.dir.join("before.epub");
    fs::copy(&copy.path, &before).unwrap();
    let ix = text_chapter(&copy.path, div_br_chapter, 40);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.js("window.__t.scrollTo(0.4)");
    frames(&mut app);
    let read_scroll = status(&mut app)["scroll"].as_f64().unwrap();
    let read_y = app.js("window.__t.textY()").as_f64().unwrap();
    app.key(u32::from('e'));
    until(&mut app, "Edit after E", IN_EDIT);
    frames(&mut app);
    let edit_y = app.js("window.__t.textY()").as_f64().unwrap();
    let edit_scroll = status(&mut app)["scroll"].as_f64().unwrap();
    println!("text moved on entering Edit: {} px", edit_y - read_y);
    assert!(
        (edit_y - read_y).abs() <= 1.0,
        "text stays still entering Edit"
    );
    let caret_y = app.js("window.__t.anchorY()").as_f64().unwrap();
    let toolbar = app.js("window.__t.status().toolbar");
    assert_eq!(toolbar, json!(true));
    app.type_text("qzjx");
    until(&mut app, "dirty after typing", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read after Esc", IN_READ);
    frames(&mut app);
    let after = status(&mut app);
    let back_y = app.js("window.__t.textY()").as_f64().unwrap();
    println!(
        "scroll read {read_scroll} edit {edit_scroll} after {} text y read {read_y} after {back_y}",
        after["scroll"]
    );
    assert!(
        (back_y - read_y).abs() <= 1.0,
        "text stays still leaving Edit"
    );
    assert_eq!(after["fail"], json!(""));
    assert_eq!(save_calls(&mut app), 1);
    let caret_after = app.js("window.__t.anchorY()").as_f64().unwrap();
    assert!(
        (caret_after - caret_y).abs() <= (edit_y - read_y).abs() + 40.0,
        "the caret stays on its line"
    );
    app.close();
    let old = chapter_bytes(&before, ix);
    let new = chapter_bytes(&copy.path, ix);
    let (was, now) = changed_region(&old, &new);
    println!("byte diff: removed {} inserted {}", was.len(), now.len());
    assert!(
        was.is_empty() && now == b"qzjx",
        "only the typed bytes changed"
    );
    assert_eq!(changed_chapters(&before, &copy.path), vec![ix]);
    round_trip_holds(&copy.path, ix, "qzjx");
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn double_click_enters_edit_at_the_clicked_character() {
    let copy = Copy::of(&generated_book("large-h1-p"), "dbl");
    for gtk_sequence in [false, true] {
        let before = copy.bytes();
        let scratch = copy.dir.join("before.epub");
        fs::write(&scratch, &before).unwrap();
        let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p"), 3);
        let mut app = copy.open();
        app.js(&format!("window.__t.mount({ix})"));
        let at = app.js("window.__t.pointOfText(0.5)");
        frames(&mut app);
        let at = if at.is_null() {
            app.js("window.__t.pointOfText(0.5)")
        } else {
            at
        };
        app.double_click(
            at["x"].as_f64().unwrap(),
            at["y"].as_f64().unwrap(),
            gtk_sequence,
        );
        until(&mut app, "Edit after double-click", IN_EDIT);
        let caret = app.js("window.__t.caret()");
        assert_eq!(caret["collapsed"], json!(true), "the caret is collapsed");
        let offset = caret["offset"].as_i64().unwrap();
        let clicked = at["at"].as_i64().unwrap();
        println!("gtk sequence {gtk_sequence}: clicked unit {clicked} caret offset {offset}");
        assert!(
            (offset - clicked).abs() <= 1,
            "the caret is at the clicked character"
        );
        app.type_text("q");
        until(&mut app, "dirty after typing", DIRTY);
        app.key(KEY_ESCAPE);
        until(&mut app, "Read after Esc", IN_READ);
        app.close();
        let (was, now) = diff_of(&scratch, &copy.path, ix);
        assert!(was == b"" && now == b"q");
    }
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn typing_then_undo_leaves_no_dirty_block_and_saves_nothing() {
    for (label, source) in [
        ("p", generated_book("large-h1-p")),
        ("div", generated_book("large-h1-div-br")),
    ] {
        let copy = Copy::of(&source, &format!("undo-{label}"));
        let before = copy.bytes();
        let ix = text_chapter(
            &copy.path,
            |bytes| has(bytes, "<br") || has(bytes, "<p>"),
            4,
        );
        let mut app = copy.open();
        app.js(&format!("window.__t.mount({ix})"));
        app.key(u32::from('e'));
        until(&mut app, "Edit", IN_EDIT);
        app.js("window.__t.caretInLongText(0.5)");
        app.type_text("abc");
        until(&mut app, "dirty", DIRTY);
        let mut undos = 0;
        while app.js(DIRTY).as_bool() == Some(true) && undos < 8 {
            app.key_with(u32::from('z'), MASK_CTRL);
            frames(&mut app);
            frames(&mut app);
            undos += 1;
        }
        println!("{label}: Ctrl+Z pressed {undos} times to return to clean");
        until(&mut app, "clean after undo", CLEAN);
        let calls = save_calls(&mut app);
        app.key(KEY_ESCAPE);
        until(&mut app, "Read", IN_READ);
        assert_eq!(
            save_calls(&mut app),
            calls,
            "an empty dirty set sends nothing"
        );
        assert_eq!(
            data_files_besides_covers(app.data_dir()),
            [place_name(app.data_dir(), &copy.path)],
            "no backup was made"
        );
        app.close();
        assert!(copy.bytes() == before, "the file is byte-identical");
    }
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn entering_and_leaving_without_typing_sends_nothing() {
    let copy = Copy::of(&generated_book("large-h1-div-br"), "noop");
    let before = copy.bytes();
    let mut app = copy.open();
    let read_y = app.js("window.__t.textY()").as_f64().unwrap();
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    frames(&mut app);
    let shift = (app.js("window.__t.textY()").as_f64().unwrap() - read_y).abs();
    println!("text moved entering Edit at the chapter top: {shift} px");
    assert!(shift <= 8.0);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    assert_eq!(save_calls(&mut app), 0);
    click_button(&mut app, "mode-edit");
    until(&mut app, "Edit by button", IN_EDIT);
    click_button(&mut app, "mode-read");
    until(&mut app, "Read by button", IN_READ);
    assert_eq!(save_calls(&mut app), 0);
    app.close();
    assert!(copy.bytes() == before);
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn enter_in_a_div_book_inserts_the_books_own_break_and_keeps_the_tail() {
    let copy = Copy::of(&generated_book("large-h1-div-br"), "enter-div");
    let scratch = copy.dir.join("before.epub");
    fs::write(&scratch, copy.bytes()).unwrap();
    let ix = text_chapter(&copy.path, div_br_chapter, 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    let before = app.js("window.__t.shape()");
    app.js("window.__t.caretInLongText(0.5)");
    app.key(KEY_ENTER);
    until(&mut app, "dirty after Enter", DIRTY);
    let after = app.js("window.__t.shape()");
    assert_eq!(
        before["blocks"], after["blocks"],
        "Enter did not split the block"
    );
    app.type_text("q");
    frames(&mut app);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    app.close();
    let old = chapter_bytes(&scratch, ix);
    let new = chapter_bytes(&copy.path, ix);
    let (was, now) = changed_region(&old, &new);
    println!(
        "div Enter byte diff: removed {} inserted {}",
        was.len(),
        now.len()
    );
    assert!(was.is_empty(), "nothing was removed");
    assert!(
        now.ends_with(b"q") && has(now, "<br"),
        "only breaks and the letter were added"
    );
    assert!(
        now.len() <= 2 * "<br/>".len() + 1,
        "at most the book's own run"
    );
    assert!(parses_as_xml(&new));
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn enter_in_a_paragraph_book_splits_and_writes_only_a_close_and_an_open() {
    let copy = Copy::of(&generated_book("large-h1-p"), "enter-p");
    let scratch = copy.dir.join("before.epub");
    fs::write(&scratch, copy.bytes()).unwrap();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    let before = app.js("window.__t.shape()");
    app.js("window.__t.caretInLongText(0.5)");
    app.key(KEY_ENTER);
    until(&mut app, "dirty after Enter", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    frames(&mut app);
    let after = app.js("window.__t.shape()");
    assert_eq!(
        after["blocks"].as_u64().unwrap(),
        before["blocks"].as_u64().unwrap() + 1,
        "the block split"
    );
    let unique = app.js("(()=>{const seen=[...window.__t.root().querySelectorAll('[data-ss-b]')].map((b)=>b.getAttribute('data-ss-b'));return seen.every((value,at)=>Number(value)===at)})()");
    assert_eq!(unique, json!(true), "blocks are renumbered after the save");
    app.key(u32::from('e'));
    until(&mut app, "Edit again", IN_EDIT);
    app.type_text("q");
    until(&mut app, "dirty again", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read again", IN_READ);
    assert_eq!(
        status(&mut app)["fail"],
        json!(""),
        "a second save after a split works"
    );
    app.close();
    let old = chapter_bytes(&scratch, ix);
    let new = chapter_bytes(&copy.path, ix);
    assert!(parses_as_xml(&new));
    let closes = |bytes: &[u8]| bytes.windows(4).filter(|w| *w == b"</p>").count();
    assert_eq!(closes(&new), closes(&old) + 1);
    assert!(
        new.len() - old.len() <= 16,
        "the split added {} bytes",
        new.len() - old.len()
    );
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn shift_enter_is_one_break_and_arrow_keys_stay_in_the_text() {
    let copy = Copy::of(&generated_book("large-h1-div-br"), "shift-enter");
    let scratch = copy.dir.join("before.epub");
    fs::write(&scratch, copy.bytes()).unwrap();
    let ix = text_chapter(&copy.path, div_br_chapter, 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    let where_before = status(&mut app)["where"].clone();
    app.key(KEY_RIGHT);
    app.js("new Promise((done)=>setTimeout(()=>done(true),200))");
    let moved = status(&mut app)["where"].clone();
    assert_ne!(
        where_before, moved,
        "the arrow turns the page while reading"
    );
    app.key(KEY_LEFT);
    until(
        &mut app,
        "back",
        &format!("window.__t.status().where==={where_before}"),
    );
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.key(KEY_RIGHT);
    app.key(KEY_LEFT);
    app.js("new Promise((done)=>setTimeout(()=>done(true),200))");
    assert_eq!(
        status(&mut app)["where"],
        where_before,
        "arrows move the caret while editing"
    );
    app.key_with(KEY_ENTER, MASK_SHIFT);
    until(&mut app, "dirty", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    app.close();
    let (was, now) = diff_of(&scratch, &copy.path, ix);
    assert!(was == b"" && now == b"<br/>", "Shift+Enter added one break");
}

const PILL: &str = "(()=>{const t=document.getElementById('toolbar');const p=t.getBoundingClientRect();const view=document.getElementById('scroller').getBoundingClientRect();const shadow=document.getElementById('book-host').shadowRoot;const [s]=document.getSelection().getComposedRanges({shadowRoots:[shadow]});let sel=null;if(s&&!s.collapsed){const r=document.createRange();r.setStart(s.startContainer,s.startOffset);r.setEnd(s.endContainer,s.endOffset);const b=r.getBoundingClientRect();sel={top:b.top,bottom:b.bottom,left:b.left,right:b.right}}return {shown:getComputedStyle(t).visibility==='visible',top:p.top,bottom:p.bottom,left:p.left,right:p.right,view:{top:view.top,left:view.left,right:view.right},sel}})()";

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn the_format_pill_floats_by_the_selection_and_hides_without_one() {
    let copy = Copy::of(&generated_book("large-h1-p"), "format-pill");
    let before = copy.bytes();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    frames(&mut app);
    assert_eq!(app.js(PILL)["shown"], json!(false), "no pill for a bare caret");
    app.js("window.__t.selectWord()");
    until(&mut app, "the pill by the selection", &format!("{PILL}.shown"));
    let pill = app.js(PILL);
    println!("pill {pill}");
    let at = |k: &str| pill[k].as_f64().unwrap();
    let sel = |k: &str| pill["sel"][k].as_f64().unwrap();
    let near = (at("bottom") <= sel("top") && sel("top") - at("bottom") <= 24.0)
        || (at("top") >= sel("bottom") && at("top") - sel("bottom") <= 24.0);
    assert!(near, "the pill sits just above or below the selection: {pill}");
    assert!(
        at("left") < sel("right") && at("right") > sel("left"),
        "the pill lines up with the selection: {pill}"
    );
    assert!(
        at("left") >= pill["view"]["left"].as_f64().unwrap() && at("right") <= pill["view"]["right"].as_f64().unwrap(),
        "the pill stays over the page: {pill}"
    );
    app.js("document.getElementById('scroller').scrollTop+=60");
    until(
        &mut app,
        "the pill following a scroll",
        &format!("Math.abs({PILL}.top-({}))>=50", at("top")),
    );
    click_button(&mut app, "tool-bold");
    until(&mut app, "dirty after bold", DIRTY);
    assert_eq!(app.js(PILL)["shown"], json!(true), "the pill stays for the next tool");
    app.js("window.__t.caretInLongText(0.5)");
    until(&mut app, "the pill gone with the selection", &format!("!{PILL}.shown"));
    app.key_with(u32::from('z'), MASK_CTRL);
    until(&mut app, "undo", CLEAN);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    assert_eq!(app.js(PILL)["shown"], json!(false), "no pill in Read");
    app.close();
    assert!(copy.bytes() == before);
}

fn tool_cases() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("tool-bold", "<b>", "bold"),
        ("tool-italic", "<i>", "italic"),
        ("tool-heading", "<h2", "h2"),
        ("tool-quote", "<blockquote", "quote"),
        ("tool-list", "<li", "list"),
    ]
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn every_toolbar_action_saves_valid_xhtml_in_both_shapes() {
    for (shape, source) in [
        ("p", generated_book("large-h1-p")),
        ("div", generated_book("large-h1-div-br")),
    ] {
        for (id, tag, label) in tool_cases() {
            let copy = Copy::of(&source, &format!("tool-{shape}-{label}"));
            let scratch = copy.dir.join("before.epub");
            fs::write(&scratch, copy.bytes()).unwrap();
            let ix = text_chapter(
                &copy.path,
                |bytes| has(bytes, "<br") || has(bytes, "<p>"),
                2,
            );
            let mut app = copy.open();
            app.js(&format!("window.__t.mount({ix})"));
            app.key(u32::from('e'));
            until(&mut app, "Edit", IN_EDIT);
            app.js("window.__t.caretInLongText(0.5)");
            app.js("window.__t.selectWord()");
            until(&mut app, "the pill by the selection", &format!("{PILL}.shown"));
            click_button(&mut app, id);
            until(&mut app, "dirty after the tool", DIRTY);
            app.key(KEY_ESCAPE);
            until(&mut app, "Read", IN_READ);
            assert_eq!(status(&mut app)["fail"], json!(""), "{shape} {label} saved");
            frames(&mut app);
            let shown = app.js("window.__t.shape()");
            app.close();
            let old = chapter_bytes(&scratch, ix);
            let new = chapter_bytes(&copy.path, ix);
            assert!(parses_as_xml(&new), "{shape} {label}: saved chapter parses");
            let old_text = visible_len(&old);
            let new_text = visible_len(&new);
            let (was, now) = changed_region(&old, &new);
            println!(
                "{shape:<4}{label:<8} removed {:>6} inserted {:>6} text {old_text}->{new_text} blocks {}",
                was.len(),
                now.len(),
                shown["blocks"]
            );
            assert!(has(&new, tag), "{shape} {label}: the tag is stored");
            assert_eq!(old_text, new_text, "{shape} {label}: no text is lost");
            assert!(
                ebook_meta_opens(&copy.path),
                "{shape} {label}: ebook-meta opens it"
            );
        }
    }
}

fn visible_len(bytes: &[u8]) -> usize {
    jadeslip_core::proof::visible_text_of(bytes.to_vec())
        .map(|text| text.as_str().split_whitespace().map(str::len).sum())
        .unwrap_or(0)
}

const TOOLBAR_STATE: &str = "(()=>{const n=window.__t.root().textContent.split(/\\s+/).filter(Boolean).length;return {label:document.getElementById('save-label').textContent,primary:document.getElementById('save').classList.contains('primary'),count:document.getElementById('word-count').textContent,expected:`${n.toLocaleString('en-US')} words`}})()";

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn the_toolbar_shows_save_state_and_the_chapter_word_count() {
    let copy = Copy::of(&generated_book("large-h1-p"), "toolbar-state");
    let before = copy.bytes();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    let clean = app.js(TOOLBAR_STATE);
    assert_eq!(clean["label"], json!("Saved"));
    assert_eq!(clean["primary"], json!(false), "a clean chapter has a quiet Save");
    assert_eq!(clean["count"], clean["expected"]);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text(" qq zz ");
    until(&mut app, "dirty", DIRTY);
    let dirty = until(
        &mut app,
        "the count to follow typing",
        &format!("(()=>{{const s={TOOLBAR_STATE};return s.label==='Save'&&s.primary&&s.count===s.expected&&s.count!=={}}})()", json!(clean["count"])),
    );
    assert_eq!(dirty, json!(true));
    app.key_with(u32::from('z'), MASK_CTRL);
    until(&mut app, "undo", CLEAN);
    until(&mut app, "the quiet Save again", &format!("(()=>{{const s={TOOLBAR_STATE};return s.label==='Saved'&&!s.primary}})()"));
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    assert_eq!(app.js("document.getElementById('word-count').textContent"), json!(""));
    app.close();
    assert!(copy.bytes() == before);
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn undo_and_redo_buttons_and_keys_work() {
    let copy = Copy::of(&generated_book("large-h1-p"), "undo-redo");
    let before = copy.bytes();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    let length = "window.__t.root().textContent.length";
    let start = app.js(length);
    app.type_text("abc");
    until(&mut app, "dirty", DIRTY);
    click_button(&mut app, "tool-undo");
    until(&mut app, "undo button", CLEAN);
    assert_eq!(app.js(length), start);
    click_button(&mut app, "tool-redo");
    until(&mut app, "redo button", DIRTY);
    app.key_with(u32::from('z'), MASK_CTRL);
    until(&mut app, "ctrl z", CLEAN);
    app.key_with(u32::from('z'), MASK_CTRL | MASK_SHIFT);
    until(&mut app, "ctrl shift z", DIRTY);
    app.key_with(u32::from('z'), MASK_CTRL);
    until(&mut app, "ctrl z", CLEAN);
    app.key(KEY_BACKSPACE);
    until(&mut app, "backspace", DIRTY);
    app.key_with(u32::from('z'), MASK_CTRL);
    until(&mut app, "undo backspace", CLEAN);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    assert_eq!(save_calls(&mut app), 0);
    app.close();
    assert!(copy.bytes() == before);
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn paste_inserts_plain_text_only() {
    let copy = Copy::of(&generated_book("large-h1-p"), "paste");
    let scratch = copy.dir.join("before.epub");
    fs::write(&scratch, copy.bytes()).unwrap();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    let handled = app.js(r#"window.__t.paste({'text/plain':'qq1<script>x</script>','text/html':'<b>RICHBOLD</b><img src="http://127.0.0.1:9/K/paste" onerror="window.__pwned=1"/>'})"#);
    assert_eq!(handled, json!(true), "the paste handler took the event");
    until(&mut app, "dirty after paste", DIRTY);
    let dom = app.js("(()=>{const root=window.__t.root();return {rich:root.textContent.includes('RICHBOLD'),plain:root.textContent.includes('qq1<script>x</script>'),images:root.querySelectorAll('img').length,bold:root.querySelectorAll('b').length,pwned:window.__t.pwned()}})()");
    println!("after rich paste {dom}");
    assert_eq!(dom["rich"], json!(false), "the rich flavour is ignored");
    assert_eq!(dom["plain"], json!(true), "the plain flavour is inserted");
    assert_eq!(dom["images"], json!(0));
    assert_eq!(dom["bold"], json!(0));
    assert_eq!(dom["pwned"], json!(0));
    app.js(r#"window.__t.paste({'text/html':'<b>ONLYHTML</b>'})"#);
    let refused = status(&mut app)["toast"].clone();
    println!("html only paste says: {refused}");
    assert_eq!(refused, json!("Only plain text can be pasted."));
    assert_eq!(
        app.js("window.__t.root().textContent.includes('ONLYHTML')"),
        json!(false)
    );
    app.js("window.__t.paste({'text/plain':'qq2\\nqq3'})");
    frames(&mut app);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    app.close();
    let old = chapter_bytes(&scratch, ix);
    let new = chapter_bytes(&copy.path, ix);
    let (was, now) = changed_region(&old, &new);
    println!(
        "paste byte diff: removed {} inserted {}",
        was.len(),
        now.len()
    );
    assert!(parses_as_xml(&new));
    assert!(
        has(&new, "qq1&lt;script&gt;x&lt;/script&gt;"),
        "the plain text is stored escaped"
    );
    assert!(!has(&new, "RICHBOLD") && !has(&new, "ONLYHTML") && !has(&new, "onerror"));
    assert!(has(&new, "qq2") && has(&new, "qq3"));
    round_trip_holds(&copy.path, ix, "qq3");
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn a_chapter_change_with_unsaved_edits_asks() {
    let copy = Copy::of(&generated_book("large-h1-p"), "chapter-prompt");
    let scratch = copy.dir.join("before.epub");
    let before = copy.bytes();
    fs::write(&scratch, &before).unwrap();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    let start = status(&mut app)["where"].clone();
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    click_button(&mut app, "next");
    until(
        &mut app,
        "a clean Edit moves on",
        &format!("window.__t.status().where!=={start}&&window.__t.status().read"),
    );
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qz");
    until(&mut app, "dirty", DIRTY);
    click_button(&mut app, "next");
    until(&mut app, "dialog", "window.__t.status().dialog");
    assert_eq!(status(&mut app)["where"], start, "no move while asking");
    app.key(KEY_ESCAPE);
    until(&mut app, "Esc keeps editing", "!window.__t.status().dialog");
    assert_eq!(status(&mut app)["editable"], json!("true"));
    assert_eq!(status(&mut app)["unsaved"], json!(true));
    app.type_text("x");
    frames(&mut app);
    click_button(&mut app, "next");
    until(&mut app, "dialog", "window.__t.status().dialog");
    click_button(&mut app, "dialog-keep");
    until(&mut app, "keep", "!window.__t.status().dialog");
    click_button(&mut app, "next");
    until(&mut app, "dialog", "window.__t.status().dialog");
    click_button(&mut app, "dialog-discard");
    until(
        &mut app,
        "discarded and moved",
        &format!("window.__t.status().where!=={start}&&window.__t.status().read"),
    );
    assert_eq!(save_calls(&mut app), 0, "discard sends nothing");
    assert!(copy.bytes() == before, "discard leaves the file alone");
    app.js(&format!("window.__t.mount({ix})"));
    assert_eq!(
        app.js("window.__t.root().textContent.includes('qzx')"),
        json!(false)
    );
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qy");
    until(&mut app, "dirty", DIRTY);
    click_button(&mut app, "next");
    until(&mut app, "dialog", "window.__t.status().dialog");
    click_button(&mut app, "dialog-save");
    until(
        &mut app,
        "saved and moved",
        &format!(
            "window.__t.status().where!=={start}&&window.__t.status().read&&!window.__t.status().unsaved"
        ),
    );
    assert_eq!(save_calls(&mut app), 1);
    app.close();
    let (was, now) = diff_of(&scratch, &copy.path, ix);
    assert!(was == b"" && now == b"qy");
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn closing_the_window_with_unsaved_edits_asks() {
    let copy = Copy::of(&generated_book("large-h1-p"), "close-prompt");
    let scratch = copy.dir.join("before.epub");
    let before = copy.bytes();
    fs::write(&scratch, &before).unwrap();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);

    let mut app = copy.open();
    app.request_close_through_the_compositor();
    assert!(
        app.wait_exit(Duration::from_secs(5)),
        "reading closes at once"
    );
    app.close();

    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.request_close_through_the_compositor();
    assert!(
        app.wait_exit(Duration::from_secs(5)),
        "a clean Edit closes without asking"
    );
    app.close();
    assert!(copy.bytes() == before);

    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qz");
    until(&mut app, "dirty", DIRTY);
    app.request_close_through_the_compositor();
    until(&mut app, "dialog on close", "window.__t.status().dialog");
    assert!(
        !app.wait_exit(Duration::from_millis(300)),
        "the window stays while asking"
    );
    click_button(&mut app, "dialog-keep");
    until(&mut app, "kept", "!window.__t.status().dialog");
    assert_eq!(status(&mut app)["unsaved"], json!(true));
    std::thread::sleep(Duration::from_secs(6));
    app.request_close_through_the_compositor();
    until(
        &mut app,
        "asks again after a long wait",
        "window.__t.status().dialog",
    );
    click_button(&mut app, "dialog-discard");
    assert!(app.wait_exit(Duration::from_secs(5)), "discard closes");
    app.close();
    assert!(copy.bytes() == before, "discard wrote nothing");

    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qy");
    until(&mut app, "dirty", DIRTY);
    app.request_close_through_the_compositor();
    until(&mut app, "dialog on close", "window.__t.status().dialog");
    click_button(&mut app, "dialog-save");
    assert!(app.wait_exit(Duration::from_secs(10)), "save then close");
    app.close();
    let (was, now) = diff_of(&scratch, &copy.path, ix);
    assert!(
        was == b"" && now == b"qy",
        "the saved text reached the file"
    );
}

fn type_and_expect_failure(app: &mut App, ix: u32, marker: &str) -> Value {
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text(marker);
    until(app, "dirty", DIRTY);
    expect_failure_keeps_text(app, marker)
}

fn expect_failure_keeps_text(app: &mut App, marker: &str) -> Value {
    let calls = save_calls(app);
    app.key(KEY_ESCAPE);
    until(app, "the failure shows", "window.__t.status().fail!==''");
    let failed = status(app);
    assert_eq!(save_calls(app), calls + 1);
    assert_eq!(failed["editable"], json!("true"), "back in Edit");
    assert_eq!(failed["edit"], json!(true));
    assert_eq!(failed["toolbar"], json!(true));
    assert_eq!(failed["unsaved"], json!(true));
    let present = app.js(&format!(
        "window.__t.root().textContent.includes({})",
        json!(marker)
    ));
    assert_eq!(present, json!(true), "the typed text is still in the DOM");
    let message = failed["fail"].as_str().unwrap().to_owned();
    println!("failure message: {message}");
    assert!(message.contains("Your text is still on screen."));
    failed
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn a_read_only_book_keeps_the_text_and_saves_once_writable() {
    let copy = Copy::of(&generated_book("large-h1-p"), "readonly");
    let before = copy.bytes();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    fs::set_permissions(&copy.path, fs::Permissions::from_mode(0o444)).unwrap();
    let failed = type_and_expect_failure(&mut app, ix, "qro");
    assert_eq!(failed["reload"], json!(false));
    assert!(copy.bytes() == before);
    fs::set_permissions(&copy.path, fs::Permissions::from_mode(0o644)).unwrap();
    app.key(KEY_ESCAPE);
    until(&mut app, "the retry saves", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    app.close();
    assert!(has(&chapter_bytes(&copy.path, ix), "qro"));
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn a_book_changed_on_disk_keeps_the_text_and_reload_recovers() {
    let copy = Copy::of(&generated_book("large-h1-p"), "changed");
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    let changed = copy.bytes();
    let temp = copy.dir.join("other.epub");
    fs::write(&temp, &changed).unwrap();
    fs::rename(&temp, &copy.path).unwrap();
    let failed = type_and_expect_failure(&mut app, ix, "qdisk");
    assert_eq!(failed["reload"], json!(true), "the bar offers a reload");
    assert!(copy.bytes() == changed, "the outside change is untouched");
    click_button(&mut app, "fail-reload");
    until(&mut app, "reload asks first", "window.__t.status().dialog");
    click_button(&mut app, "dialog-keep");
    until(&mut app, "keep", "!window.__t.status().dialog");
    let kept = app.js("window.__t.root().textContent.includes('qdisk')");
    assert_eq!(kept, json!(true));
    click_button(&mut app, "fail-reload");
    until(&mut app, "reload asks first", "window.__t.status().dialog");
    app.js("window.__t.root().dataset.beforeReload='1'");
    click_button(&mut app, "dialog-discard");
    until(
        &mut app,
        "reloaded",
        "window.__t.status().read&&window.__t.status().fail===''&&window.__t.root()?.dataset.beforeReload!=='1'",
    );
    frames(&mut app);
    app.key(u32::from('e'));
    until(&mut app, "Edit after reload", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qok");
    until(&mut app, "dirty", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "saved after reload", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    app.close();
    assert!(has(&chapter_bytes(&copy.path, ix), "qok"));
}

const REGAIN_FOCUS: &str = "window.dispatchEvent(new FocusEvent('focus'))";

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn a_book_changed_on_disk_is_flagged_when_the_window_regains_focus() {
    let copy = Copy::of(&generated_book("large-h1-p"), "focus");
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    frames(&mut app);
    app.js(REGAIN_FOCUS);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        status(&mut app)["fail"],
        json!(""),
        "an unchanged book raised the bar"
    );
    let first = app.js("window.__t.root().querySelector('[data-ss-b]').textContent");

    let data = copy.dir.join("outside-data");
    let outside = Book::open(&copy.path, &data, BookKey(9), DEFAULT_LIMITS).unwrap();
    let rendered = outside.read().render(ChapterIx(ix)).unwrap();
    outside
        .save_chapter(
            ChapterIx(ix),
            &BlockEdits {
                base: rendered.rev,
                changes: vec![BlockChange::Delete { of: BlockIx(0) }],
            },
        )
        .unwrap();
    drop(outside);

    app.js(REGAIN_FOCUS);
    until(&mut app, "the bar after focus", "window.__t.status().reload");
    let flagged = status(&mut app);
    assert_eq!(flagged["read"], json!(true));
    assert!(
        flagged["fail"].as_str().unwrap().contains("changed on disk"),
        "{flagged}"
    );
    click_button(&mut app, "fail-reload");
    until(
        &mut app,
        "the reload shows the outside change",
        &format!(
            "window.__t.status().fail===''&&window.__t.root()?.querySelector('[data-ss-b]')?.textContent!=={first}"
        ),
    );
    frames(&mut app);
    app.key(u32::from('e'));
    until(&mut app, "Edit after reload", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qfocus");
    until(&mut app, "dirty", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "saved after reload", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    app.js(REGAIN_FOCUS);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        status(&mut app)["fail"],
        json!(""),
        "the app's own save raised the bar"
    );
    let check = app.js("(async()=>{const at=performance.now();await window.__t.ipc.changedOnDisk();return performance.now()-at})()");
    println!("one disk check took {check} ms");
    app.close();
    assert!(has(&chapter_bytes(&copy.path, ix), "qfocus"));
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn a_stale_plan_keeps_the_text() {
    let copy = Copy::of(&generated_book("large-h1-p"), "stale");
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qstale");
    until(&mut app, "dirty", DIRTY);
    let data = copy.dir.join("outside-data");
    let outside = Book::open(&copy.path, &data, BookKey(9), DEFAULT_LIMITS).unwrap();
    let rendered = outside.read().render(ChapterIx(ix)).unwrap();
    outside
        .save_chapter(
            ChapterIx(ix),
            &BlockEdits {
                base: rendered.rev,
                changes: vec![BlockChange::Delete { of: BlockIx(0) }],
            },
        )
        .unwrap();
    app.js("window.__t.ipc.reloadBook().then(()=>true)");
    let bytes = copy.bytes();
    let failed = expect_failure_keeps_text(&mut app, "qstale");
    assert_eq!(failed["reload"], json!(true));
    assert!(copy.bytes() == bytes);
    app.close();
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn a_refused_edit_keeps_the_text_and_says_so() {
    let copy = Copy::of(&generated_book("large-h1-p"), "wouldbreak");
    let before = copy.bytes();
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.op(&json!({"op": "fail_save", "kind": "wouldBreak"}));
    let failed = type_and_expect_failure(&mut app, ix, "qbreak");
    assert_eq!(failed["reload"], json!(false));
    assert!(copy.bytes() == before);
    app.op(&json!({"op": "fail_save", "kind": null}));
    app.key(KEY_ESCAPE);
    until(&mut app, "the retry saves", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    app.close();
    assert!(has(&chapter_bytes(&copy.path, ix), "qbreak"));
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn a_full_disk_keeps_the_text_and_saves_once_there_is_room() {
    let source = generated_book("toc-none");
    let mount = scratch_dir().join(format!("full-{}", std::process::id()));
    let _ = fs::remove_dir_all(&mount);
    fs::create_dir_all(&mount).unwrap();
    let script = format!(
        "mount -t tmpfs -o size=4m tmpfs '{m}' && cp '{s}' '{m}/book.epub' && (dd if=/dev/zero of='{m}/pad' bs=4k 2>/dev/null; true) && exec '{a}' '{m}/book.epub'",
        m = mount.display(),
        s = source.display(),
        a = jadeslip_e2e::e2e_binary().display(),
    );
    let mut command = Command::new("unshare");
    command.args(["-Urm", "sh", "-c", &script]);
    let mut app = App::spawn(command, &[]);
    app.wait_first_text();
    app.assert_on_headless_output();
    app.js(INSTALL);
    let inner =
        PathBuf::from(format!("/proc/{}/root", app.pid)).join(mount.strip_prefix("/").unwrap());
    let before = fs::read(inner.join("book.epub")).unwrap();
    let ix = text_chapter(
        &inner.join("book.epub"),
        |bytes| has(bytes, "<p>") || has(bytes, "<br"),
        0,
    );
    type_and_expect_failure(&mut app, ix, "qfull");
    assert!(
        fs::read(inner.join("book.epub")).unwrap() == before,
        "the book is unchanged"
    );
    let leftovers: Vec<_> = fs::read_dir(&inner)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "no temp file stays behind");
    fs::remove_file(inner.join("pad")).unwrap();
    app.key(KEY_ESCAPE);
    until(&mut app, "the retry saves", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    assert!(has(&chapter_bytes(&inner.join("book.epub"), ix), "qfull"));
    app.close();
    let _ = fs::remove_dir_all(&mount);
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn an_image_page_refuses_edit() {
    let copy = Copy::of(&generated_book("images-only"), "image");
    let before = copy.bytes();
    let mut app = copy.open();
    app.key(u32::from('e'));
    until(
        &mut app,
        "the refusal shows",
        "window.__t.status().toast!==''",
    );
    assert_eq!(status(&mut app)["read"], json!(true));
    assert_eq!(status(&mut app)["editable"], Value::Null);
    println!("image page says: {}", status(&mut app)["toast"]);
    app.close();
    assert!(copy.bytes() == before);
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn the_second_save_in_a_row_uses_the_returned_rev_and_ctrl_s_stays_in_edit() {
    let copy = Copy::of(&generated_book("large-h1-p"), "second");
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qa");
    until(&mut app, "dirty", DIRTY);
    app.key_with(u32::from('s'), MASK_CTRL);
    until(&mut app, "saved in Edit", CLEAN);
    assert_eq!(status(&mut app)["editable"], json!("true"));
    assert_eq!(status(&mut app)["fail"], json!(""));
    assert_eq!(
        status(&mut app)["toast"],
        json!("Saved. Your original is backed up.")
    );
    app.type_text("qb");
    until(&mut app, "dirty again", DIRTY);
    app.key_with(u32::from('s'), MASK_CTRL);
    until(&mut app, "saved again", CLEAN);
    assert_eq!(status(&mut app)["fail"], json!(""));
    assert!(
        has(&chapter_bytes(&copy.path, ix), "qaqb"),
        "both saves landed at the caret"
    );
    app.type_text("qc");
    until(&mut app, "dirty third", DIRTY);
    app.key_with(u32::from('z'), MASK_CTRL);
    frames(&mut app);
    frames(&mut app);
    let shown = app.js("window.__t.root().textContent.includes('qaqb')");
    println!("one Ctrl+Z after two saves leaves the earlier typing on screen: {shown}");
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    assert_eq!(status(&mut app)["fail"], json!(""));
    app.close();
    assert_eq!(
        has(&chapter_bytes(&copy.path, ix), "qaqb"),
        shown == json!(true),
        "the file matches the screen after an undo across saves"
    );
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn typing_during_a_slow_save_is_kept_and_a_second_esc_starts_no_second_save() {
    let copy = Copy::of(&generated_book("large-h1-p"), "slow");
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.op(&json!({"op": "fail_save", "kind": "delay"}));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("qa");
    until(&mut app, "dirty", DIRTY);
    app.key_with(u32::from('s'), MASK_CTRL);
    app.type_text("qb");
    until(
        &mut app,
        "the typing shows after the save",
        "window.__t.root().textContent.includes('qaqb')",
    );
    until(&mut app, "dirty from the queued typing", DIRTY);
    assert_eq!(save_calls(&mut app), 1);
    app.key(KEY_ESCAPE);
    app.key(KEY_ESCAPE);
    app.key(KEY_ESCAPE);
    assert_eq!(status(&mut app)["read"], json!(true), "Read shows at once");
    until(&mut app, "Read", IN_READ);
    assert_eq!(save_calls(&mut app), 2, "repeated Esc starts one save");
    app.close();
    assert!(has(&chapter_bytes(&copy.path, ix), "qaqb"));
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn selecting_everything_and_retyping_saves_valid_xml_in_both_shapes() {
    for (label, source) in [
        ("p", generated_book("large-h1-p")),
        ("div", generated_book("large-h1-div-br")),
    ] {
        let copy = Copy::of(&source, &format!("wipe-{label}"));
        let ix = text_chapter(
            &copy.path,
            |bytes| has(bytes, "<br") || has(bytes, "<p>"),
            2,
        );
        let mut app = copy.open();
        app.js(&format!("window.__t.mount({ix})"));
        app.key(u32::from('e'));
        until(&mut app, "Edit", IN_EDIT);
        app.js("(()=>{const root=window.__t.root();const walker=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);let first=null,last=null;while(walker.nextNode()){first=first??walker.currentNode;last=walker.currentNode}document.getSelection().setBaseAndExtent(first,0,last,last.data.length);return true})()");
        app.key(KEY_BACKSPACE);
        until(&mut app, "dirty", DIRTY);
        app.type_text("qwipe");
        frames(&mut app);
        app.key(KEY_ESCAPE);
        until(
            &mut app,
            "saved or refused",
            "(()=>{const s=window.__t.status();return (s.read&&!s.unsaved)||s.fail!==''})()",
        );
        let after = status(&mut app);
        println!(
            "{label}: fail {} shape {}",
            after["fail"],
            app.js("window.__t.shape()")
        );
        assert_eq!(after["fail"], json!(""));
        let numbered = app.js("(()=>{const seen=[...window.__t.root().querySelectorAll('[data-ss-b]')].map((b)=>Number(b.getAttribute('data-ss-b')));return seen.every((value,at)=>value===at)})()");
        assert_eq!(numbered, json!(true));
        app.key(u32::from('e'));
        until(&mut app, "Edit again", IN_EDIT);
        app.type_text("qagain");
        until(&mut app, "dirty again", DIRTY);
        app.key(KEY_ESCAPE);
        until(&mut app, "Read again", IN_READ);
        assert_eq!(status(&mut app)["fail"], json!(""));
        app.close();
        let saved = chapter_bytes(&copy.path, ix);
        assert!(parses_as_xml(&saved));
        assert!(has(&saved, "qwipe") && has(&saved, "qagain"));
        round_trip_holds(&copy.path, ix, "qagain");
    }
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn opening_and_saving_never_touches_the_real_data_folder() {
    let real =
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/jadeslip"));
    let listing = |dir: &Option<PathBuf>| -> Option<Vec<(String, u64)>> {
        let dir = dir.as_ref()?;
        let mut rows: Vec<(String, u64)> = fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|entry| {
                Some((
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.metadata().ok()?.len(),
                ))
            })
            .collect();
        rows.sort();
        Some(rows)
    };
    let before = listing(&real);
    let copy = Copy::of(&generated_book("large-h1-p"), "data-dir");
    let ix = text_chapter(&copy.path, |bytes| has(bytes, "<p>"), 2);
    let mut app = copy.open();
    app.js(&format!("window.__t.mount({ix})"));
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    app.js("window.__t.caretInLongText(0.5)");
    app.type_text("q");
    until(&mut app, "dirty", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "Read", IN_READ);
    assert!(
        app.data_dir().join("books").is_dir(),
        "the backup is under the override"
    );
    app.close();
    assert_eq!(listing(&real), before);
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

const TIMER: &str = r"(()=>{
  window.__timing = { marks: [] };
  const twoFrames = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  document.addEventListener('keydown', (event) => {
    const start = performance.now();
    if (event.key === 'q') {
      twoFrames().then(() => window.__timing.marks.push({ kind: 'typed', painted: performance.now() - start }));
      return;
    }
    if (event.key !== 'e' && event.key !== 'Escape') return;
    const kind = event.key === 'e' ? 'enter' : 'leave';
    const unsaved = !document.getElementById('unsaved').hidden;
    queueMicrotask(() => {
      const looked = performance.now();
      twoFrames().then(() => window.__timing.marks.push({ kind, unsaved, sync: looked - start, painted: performance.now() - start }));
    });
    if (kind === 'leave' && unsaved) {
      const watcher = new MutationObserver(() => {
        if (document.getElementById('unsaved').hidden) {
          watcher.disconnect();
          window.__timing.marks.push({ kind: 'saved', ms: performance.now() - start });
        }
      });
      watcher.observe(document.getElementById('unsaved'), { attributes: true });
    }
  }, true);
  return true;
})()";

#[test]
#[ignore = "GUI and real books: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn measures_edit_latency_on_a_copy_of_the_largest_real_book() {
    let source = largest_real_book();
    let copy = Copy::of(&source, "timing");
    let size = copy.bytes().len();
    let div = text_chapter(&copy.path, div_br_chapter, 100);
    let reader = Reader::open(File::open(&copy.path).unwrap(), DEFAULT_LIMITS).unwrap();
    let nav = reader
        .rows()
        .iter()
        .find(|row| row.is_nav)
        .map(|row| row.ix.0)
        .unwrap();
    println!("book {} MB chapters {}", size >> 20, reader.rows().len());
    let mut app = copy.open();
    app.js(TIMER);
    for (label, ix, cycles) in [("div chapter", div, 25), ("nav 710 KB", nav, 7)] {
        app.js(&format!("window.__t.mount({ix})"));
        frames(&mut app);
        let (mut enter, mut leave_clean, mut read_look, mut saved, mut typed) =
            (vec![], vec![], vec![], vec![], vec![]);
        for _ in 0..cycles {
            app.js("(window.__timing.marks = [], true)");
            app.key(u32::from('e'));
            until(&mut app, "Edit", IN_EDIT);
            frames(&mut app);
            app.key(KEY_ESCAPE);
            until(&mut app, "Read", IN_READ);
            frames(&mut app);
            frames(&mut app);
            app.key(u32::from('e'));
            until(&mut app, "Edit", IN_EDIT);
            frames(&mut app);
            app.js("window.__t.caretInLongText(0.5)");
            app.type_text("q");
            until(&mut app, "dirty", DIRTY);
            app.key(KEY_ESCAPE);
            until(&mut app, "Read", IN_READ);
            assert_eq!(status(&mut app)["fail"], json!(""));
            frames(&mut app);
            frames(&mut app);
            let marks = app.js("window.__timing.marks");
            for mark in marks.as_array().unwrap() {
                match (mark["kind"].as_str().unwrap(), mark["unsaved"].as_bool()) {
                    ("enter", _) => enter.push(mark["painted"].as_f64().unwrap()),
                    ("leave", Some(false)) => leave_clean.push(mark["painted"].as_f64().unwrap()),
                    ("leave", Some(true)) => read_look.push(mark["painted"].as_f64().unwrap()),
                    ("saved", _) => saved.push(mark["ms"].as_f64().unwrap()),
                    ("typed", _) => typed.push(mark["painted"].as_f64().unwrap()),
                    _ => {}
                }
            }
        }
        println!(
            "{label:<12} key to second frame, ms: enter median {:.1} max {:.1} | typed key median {:.1} | Esc clean median {:.1} | Esc dirty read look median {:.1} | Esc to saved median {:.1} max {:.1} | samples {} {} {} {}",
            median(&mut enter),
            enter.iter().cloned().fold(0.0, f64::max),
            median(&mut typed),
            median(&mut leave_clean),
            median(&mut read_look),
            median(&mut saved),
            saved.iter().cloned().fold(0.0, f64::max),
            enter.len(),
            leave_clean.len(),
            read_look.len(),
            saved.len(),
        );
        assert!(!saved.is_empty());
    }
    app.close();
}

#[test]
#[ignore = "GUI and real books: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn enter_on_real_chapters_changes_only_what_it_adds() {
    let mut books = real_books();
    books.sort_by_key(|book| fs::metadata(book).map_or(0, |meta| meta.len()));
    let largest = largest_real_book();
    for source in books.iter().filter(|book| {
        **book == largest
            || book
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("tech_b.epub"))
    }) {
        let label = jadeslip_e2e::book_label(source);
        let copy = Copy::of(source, &format!("sweep-{label}"));
        let scratch = copy.dir.join("before.epub");
        fs::write(&scratch, copy.bytes()).unwrap();
        let reader = Reader::open(File::open(&scratch).unwrap(), DEFAULT_LIMITS).unwrap();
        let count = reader.rows().len() as u32;
        let step = (count / 60).max(1);
        let mut app = copy.open();
        let (
            mut tried,
            mut only_breaks,
            mut split,
            mut other,
            mut largest_change,
            mut invalid,
            mut failed,
        ) = (0u32, 0u32, 0u32, 0u32, 0usize, 0u32, 0u32);
        let mut repaired = 0u32;
        let mut book_run = 0u32;
        for ix in (0..count).step_by(step as usize) {
            if reader.rows()[ix as usize].is_nav {
                continue;
            }
            let Ok(Extracted::Document(_)) = reader.extract(ChapterIx(ix)) else {
                continue;
            };
            let old = chapter_bytes(&copy.path, ix);
            app.js(&format!("window.__t.mount({ix})"));
            let long = app.js("window.__t.shape().text > 200");
            if long != json!(true) {
                continue;
            }
            app.key(u32::from('e'));
            until(&mut app, "Edit", IN_EDIT);
            if app
                .try_js("window.__t.caretInLongText(0.5)", 5_000)
                .is_err()
            {
                app.key(KEY_ESCAPE);
                until(&mut app, "Read", IN_READ);
                continue;
            }
            app.key(KEY_ENTER);
            until(&mut app, "dirty", DIRTY);
            app.key(KEY_ESCAPE);
            until(
                &mut app,
                "saved or failed",
                "(()=>{const s=window.__t.status();return (s.read&&!s.unsaved)||s.fail!==''})()",
            );
            if status(&mut app)["fail"] != json!("") {
                failed += 1;
                app.key(KEY_ESCAPE);
                continue;
            }
            tried += 1;
            let new = chapter_bytes(&copy.path, ix);
            if !parses_as_xml(&new) {
                invalid += 1;
            }
            if !parses_as_xml(&old) {
                repaired += 1;
                continue;
            }
            let (was, now) = changed_region(&old, &new);
            largest_change = largest_change.max(was.len().max(now.len()));
            let only_br = was.is_empty()
                && !now.is_empty()
                && String::from_utf8_lossy(now).replace("<br/>", "").is_empty();
            let looks_split = was.len() <= 1 && now.len() < 128 && has(now, "</");
            if only_br {
                only_breaks += 1;
                if now == b"<br/><br/>" {
                    book_run += 1;
                }
            } else if looks_split {
                split += 1;
            } else {
                other += 1;
            }
        }
        app.close();
        println!(
            "{label:<14} enter tried {tried} only_breaks {only_breaks} exactly_the_books_run {book_run} close_open {split} other {other} first_save_repaired_a_malformed_source {repaired} failed {failed} invalid_xml {invalid} largest_changed_bytes {largest_change}"
        );
        assert_eq!(invalid + failed, 0);
        assert_eq!(
            book_run, only_breaks,
            "Enter inserts the book's two-break run"
        );
        assert!(tried > 0);
        assert!(
            largest_change < 128,
            "{label}: Enter rewrote {largest_change} bytes"
        );
        assert!(ebook_meta_opens(&copy.path));
    }
}

const RAIL_TITLES: &str = "[...document.querySelectorAll('#toc-spacer .row:not([hidden])')].sort((a,b)=>Number(a.dataset.entry)-Number(b.dataset.entry)).map((r)=>r.querySelector('.t').textContent)";

fn retype_start_of(app: &mut App, selector: &str, text: &str) {
    let picked = app.js(&format!("(()=>{{const n=window.__t.root().querySelector({});const w=document.createTreeWalker(n,NodeFilter.SHOW_TEXT);w.nextNode();const t=w.currentNode;document.getSelection().setBaseAndExtent(t,0,t,Math.min(t.data.length,4));return true}})()", json!(selector)));
    assert_eq!(picked, json!(true));
    app.type_text(text);
}

fn first_rail_title_has(app: &mut App, text: &str) -> bool {
    app.js(RAIL_TITLES)[0]
        .as_str()
        .is_some_and(|title| title.contains(text))
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn editing_the_contents_page_updates_the_sidebar() {
    let source = real_books()
        .into_iter()
        .find(|path| path.to_string_lossy().contains("small_a"))
        .expect("a real book with its contents page in the spine");
    let copy = Copy::of(&source, "contents-page");
    let reader = Reader::open(File::open(&copy.path).unwrap(), DEFAULT_LIMITS).unwrap();
    let nav = reader.rows().iter().find(|row| row.is_nav).unwrap().ix.0;
    drop(reader);
    let mut app = copy.open();
    until(&mut app, "the contents", &format!("{RAIL_TITLES}.length>0"));
    app.js(&format!("window.__t.mount({nav})"));
    frames(&mut app);
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    retype_start_of(&mut app, "a", "Qzlabel");
    until(&mut app, "dirty", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "saved", IN_READ);
    let fresh = app.js("(async()=>(await window.__t.ipc.loadToc()).entries[0].label.includes('Qzlabel'))()");
    assert_eq!(fresh, json!(true), "the save changed the book's contents");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !first_rail_title_has(&mut app, "Qzlabel") {
        assert!(
            Instant::now() < deadline,
            "the sidebar kept the old label"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    app.close();
}

#[test]
#[ignore = "GUI: SS_E2E_TEST=edit tests/e2e/run.sh"]
fn editing_a_heading_in_a_book_without_contents_updates_the_sidebar() {
    let copy = Copy::of(&generated_book("toc-none"), "heading-title");
    let mut app = copy.open();
    until(
        &mut app,
        "the titles",
        "!document.getElementById('count').textContent.includes('Titling')&&document.getElementById('count').textContent.includes('entries')",
    );
    app.js("window.__t.mount(0)");
    frames(&mut app);
    app.key(u32::from('e'));
    until(&mut app, "Edit", IN_EDIT);
    retype_start_of(&mut app, "h1,h2,h3", "Qzhead");
    until(&mut app, "dirty", DIRTY);
    app.key(KEY_ESCAPE);
    until(&mut app, "saved", IN_READ);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !first_rail_title_has(&mut app, "Qzhead") {
        assert!(
            Instant::now() < deadline,
            "the sidebar kept the old heading: {}",
            app.js(RAIL_TITLES)[0]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let title = app.js("document.getElementById('where-title').textContent");
    assert!(
        title.as_str().unwrap().contains("Qzhead"),
        "the title bar kept the old heading: {title}"
    );
    app.close();
}
