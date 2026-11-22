use serde_json::{Value, json};
use jadeslip_e2e::{App, KEY_ENTER, KEY_ESCAPE, generated_book, real_books, scratch_dir};
use jadeslip_fixtures::spelling::{self, BRITISH_IN_PROSE, PLANTED};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(30);
const KEY_E: u32 = 0x65;
const KEY_Z: u32 = 0x7a;
const MASK_CTRL: u32 = 1 << 2;

const INSTALL: &str = r"(async()=>{
  const main = await import('/main.js');
  const element = (id) => document.getElementById(id);
  const host = element('book-host');
  const body = () => host.shadowRoot.querySelector('.ss-body');
  const ranges = () => [...(CSS.highlights.get('ss-spell')?.values() ?? [])];
  const centre = (box) => ({ x: box.left + box.width / 2, y: box.top + box.height / 2 });
  window.__s = {
    go: (ix) => main.goTo(ix),
    shown: () => main.shownChapter(),
    words: () => ranges().map((range) => range.toString()).sort(),
    has: () => CSS.highlights.has('ss-spell'),
    wordAt: (word) => {
      const range = ranges().find((candidate) => candidate.toString() === word);
      if (!range) return null;
      range.startContainer.parentElement.scrollIntoView({ block: 'center' });
      return centre(range.getBoundingClientRect());
    },
    dom: () => host.shadowRoot.querySelector('.ss-book').innerHTML,
    text: () => body().textContent,
    menu: () => ({
      open: !element('spell-menu').hidden,
      items: [...element('spell-menu').querySelectorAll('button')].map((button) => button.textContent),
      empty: element('spell-menu').querySelector('p')?.textContent ?? null,
    }),
    item: (text) => {
      const button = [...element('spell-menu').querySelectorAll('button')].find((row) => row.textContent === text);
      return button ? centre(button.getBoundingClientRect()) : null;
    },
    mode: () => (element('mode-edit').classList.contains('on') ? 'edit' : 'read'),
    toast: () => (element('toast').hidden ? '' : element('toast').textContent),
    switchOn: () => element('prefs-spelling-on').classList.contains('on'),
    languages: () => (element('spelling-language').hidden ? null : {
      labels: [...element('spelling-languages').querySelectorAll('button')].map((button) => button.textContent),
      chosen: element('spelling-languages').querySelector('button.on')?.textContent ?? null,
      hint: element('spelling-auto').textContent,
    }),
    languageButton: (label) => {
      const button = [...element('spelling-languages').querySelectorAll('button')].find((row) => row.textContent === label);
      if (!button) return null;
      button.scrollIntoView({ block: 'nearest' });
      return centre(button.getBoundingClientRect());
    },
    unsaved: () => !element('unsaved').hidden,
    offer: () => (element('fix-offer').hidden ? '' : element('fix-offer-text').textContent),
    offerReady: () => !element('fix-offer-fix').disabled,
    painted: () => performance.getEntriesByName('ss-spell-painted').map((mark) => mark.detail),
    firstTextAt: () => performance.getEntriesByName('ss-first-text')[0]?.startTime ?? null,
    paintedAt: () => performance.getEntriesByName('ss-spell-painted')[0]?.startTime ?? null,
    clearMarks: () => performance.clearMarks('ss-spell-painted'),
    contextMenuOn: (word) => {
      const range = ranges().find((candidate) => candidate.toString() === word);
      const { x, y } = centre(range.getBoundingClientRect());
      const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y });
      host.dispatchEvent(event);
      return event.defaultPrevented;
    },
    oldRevision: async () => {
      const ipc = await import('/ipc.js');
      const chapter = await ipc.renderChapter(main.shownChapter());
      const fresh = await ipc.spellChapter(chapter.ix, chapter.rev);
      const old = await ipc.spellChapter(chapter.ix, { ...chapter.rev, crc32: chapter.rev.crc32 + 1 });
      return { fresh: fresh.state, old: old.state };
    },
    replaceText: (needle, text) => {
      const walker = document.createTreeWalker(body(), NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const at = walker.currentNode.data.indexOf(needle);
        if (at >= 0) {
          document.getSelection().setBaseAndExtent(walker.currentNode, at, walker.currentNode, at + needle.length);
          document.execCommand('insertText', false, text);
          return true;
        }
      }
      return false;
    },
    typeAt: (word, text) => {
      const range = ranges().find((candidate) => candidate.toString() === word);
      document.getSelection().setBaseAndExtent(range.startContainer, range.startOffset, range.endContainer, range.endOffset);
      document.execCommand('insertText', false, text);
    },
    caretAfter: (needle) => {
      const walker = document.createTreeWalker(body(), NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const at = walker.currentNode.data.indexOf(needle);
        if (at >= 0) {
          const end = at + needle.length;
          document.getSelection().setBaseAndExtent(walker.currentNode, end, walker.currentNode, end);
          return true;
        }
      }
      return false;
    },
  };
  return true;
})()";

static NEXT: AtomicU32 = AtomicU32::new(0);

fn unique_dir(label: &str) -> PathBuf {
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = scratch_dir().join(format!("spelling-{}-{label}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn spelling_book() -> PathBuf {
    let path = unique_dir("book").join("spelling.epub");
    spelling::write_spelling_book(&path).expect("book");
    path
}

fn open(book: &Path) -> App {
    let mut app = App::launch(Some(book), &[]);
    ready(&mut app);
    app
}

fn ready(app: &mut App) {
    app.wait_first_text();
    assert_eq!(app.js(INSTALL), json!(true));
}

fn wait_until(app: &mut App, what: &str, mut done: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done(app) {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn words(app: &mut App) -> Vec<String> {
    serde_json::from_value(app.js("window.__s.words()")).expect("words")
}

fn wait_words(app: &mut App, wanted: &[&str]) {
    let mut wanted: Vec<String> = wanted.iter().map(|word| (*word).to_owned()).collect();
    wanted.sort();
    let deadline = Instant::now() + WAIT;
    loop {
        let now = words(app);
        if now == wanted {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "marks {now:?} never became {wanted:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn go(app: &mut App, ordinal: u32) {
    app.js(&format!(
        "(window.__s.clearMarks(), window.__s.go({ordinal}))"
    ));
    wait_until(app, "the chapter is shown", |app| {
        app.js("window.__s.shown()") == json!(ordinal)
    });
    wait_until(app, "the chapter was checked", |app| {
        app.js("window.__s.painted().length").as_u64() != Some(0)
    });
}

fn prose_marks() -> Vec<&'static str> {
    let mut wanted: Vec<&str> = planted_in(spelling::PROSE);
    wanted.extend(BRITISH_IN_PROSE);
    wanted
}

fn planted_in(chapter: u32) -> Vec<&'static str> {
    PLANTED
        .iter()
        .filter(|plant| plant.chapter == chapter)
        .map(|plant| plant.wrong)
        .collect()
}

fn point(value: &Value) -> (f64, f64) {
    (
        value["x"].as_f64().expect("x"),
        value["y"].as_f64().expect("y"),
    )
}

fn enter_edit(app: &mut App) {
    app.key(KEY_E);
    wait_until(app, "edit mode", |app| {
        app.js("window.__s.mode()") == json!("edit")
    });
}

fn open_menu_on(app: &mut App, word: &str) -> Value {
    let at = app.js(&format!("window.__s.wordAt({})", json!(word)));
    assert!(!at.is_null(), "the word is not marked");
    std::thread::sleep(Duration::from_millis(80));
    let at = app.js(&format!("window.__s.wordAt({})", json!(word)));
    let (x, y) = point(&at);
    app.right_click(x, y);
    wait_until(app, "the spelling menu opens", |app| {
        app.js("window.__s.menu().open") == json!(true)
    });
    app.js("window.__s.menu()")
}

fn click_item(app: &mut App, text: &str) {
    let at = app.js(&format!("window.__s.item({})", json!(text)));
    assert!(!at.is_null(), "the menu has no such item");
    let (x, y) = point(&at);
    app.click(x, y);
}

fn pixels(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(
        fs::File::open(path).expect("snapshot"),
    ));
    let mut reader = decoder.read_info().expect("png header");
    let mut bytes = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut bytes).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Rgb, "snapshot colour type");
    bytes.truncate(info.buffer_size());
    bytes
}

fn sha(path: &Path) -> String {
    jadeslip_fixtures::sha256_file(path).expect("hash")
}

#[test]
fn both_handler_lists_carry_the_spelling_commands() {
    let main =
        fs::read_to_string(jadeslip_e2e::repo_root().join("app/src/main.rs")).expect("main.rs");
    for command in [
        "spell::spell_chapter",
        "spell::spell_block",
        "spell::spell_suggest",
        "spell::spell_add",
        "spell::spell_languages",
        "spell::spell_language_set",
    ] {
        assert_eq!(main.matches(command).count(), 2, "{command}");
    }
}

#[test]
#[ignore = "real app on a headless output"]
fn planted_typos_are_marked_on_mount_without_touching_the_dom_or_the_book() {
    let book = spelling_book();
    let before = sha(&book);
    let mut command = Command::new(jadeslip_e2e::e2e_binary());
    command
        .arg(&book)
        .env("JADESLIP_E2E_SPELL_DELAY_MS", "700");
    let mut app = App::spawn(command, &[]);
    ready(&mut app);
    assert_eq!(app.js("window.__s.has()"), json!(false));
    let unmarked = app.js("window.__s.dom()");
    assert!(unmarked.as_str().is_some_and(|dom| !dom.is_empty()));
    wait_words(&mut app, &prose_marks());
    assert_eq!(
        app.js("window.__s.dom()"),
        unmarked,
        "marking changed the DOM"
    );
    let rows = [
        (spelling::LINES, planted_in(spelling::LINES)),
        (spelling::AGAIN, planted_in(spelling::AGAIN)),
        (spelling::BARE, planted_in(spelling::BARE)),
    ];
    for (ordinal, wanted) in rows {
        go(&mut app, ordinal);
        wait_words(&mut app, &wanted);
    }
    for ordinal in [spelling::FORMS, spelling::SCRIPTS, spelling::TAGGED] {
        go(&mut app, ordinal);
        assert_eq!(words(&mut app), Vec::<String>::new(), "chapter {ordinal}");
    }
    go(&mut app, spelling::FRENCH_UNTAGGED);
    let deadline = Instant::now() + WAIT;
    let french = loop {
        let now = words(&mut app).len();
        if now > 0 || Instant::now() >= deadline {
            break now;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(french > 0, "the control row marks untagged French");
    println!("untagged French marks in the real app: {french}");
    app.assert_on_headless_output();
    app.close();
    assert_eq!(sha(&book), before, "marking changed the book file");
}

#[test]
#[ignore = "real app on a headless output"]
fn the_marks_are_painted_on_screen() {
    let book = spelling_book();
    let mut app = open(&book);
    wait_words(&mut app, &prose_marks());
    std::thread::sleep(Duration::from_millis(100));
    let root = unique_dir("shot");
    app.snapshot_png(&root.join("marked.png"));
    app.js("CSS.highlights.delete('ss-spell')");
    std::thread::sleep(Duration::from_millis(100));
    app.snapshot_png(&root.join("plain.png"));
    let marked = pixels(&root.join("marked.png"));
    let plain = pixels(&root.join("plain.png"));
    assert_eq!(marked.len(), plain.len());
    let changed = marked
        .chunks(3)
        .zip(plain.chunks(3))
        .filter(|(with, without)| with != without)
        .count();
    let red = marked
        .chunks(3)
        .zip(plain.chunks(3))
        .filter(|(with, without)| with != without && with[0] > with[1].saturating_add(40))
        .count();
    println!("pixels changed by the marks {changed}, of those reddish {red}");
    assert!(red > 200, "the wavy underline did not paint: {red}");
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_slow_check_does_not_hold_back_first_text() {
    let book = generated_book("large-h1-p");
    let mut command = Command::new(jadeslip_e2e::e2e_binary());
    command
        .arg(&book)
        .env("JADESLIP_E2E_SPELL_DELAY_MS", "2500");
    let mut app = App::spawn(command, &[]);
    let started = app.spawned_at_ms;
    let shown = app.wait_first_text();
    let first_text = shown - started;
    assert_eq!(app.js(INSTALL), json!(true));
    assert_eq!(
        app.js("window.__s.has()"),
        json!(false),
        "marks came before first text"
    );
    wait_until(&mut app, "the delayed check finishes", |app| {
        app.js("window.__s.painted().length").as_u64() == Some(1)
    });
    let first_at = app.js("window.__s.firstTextAt()").as_f64().expect("first");
    let painted_at = app.js("window.__s.paintedAt()").as_f64().expect("painted");
    println!(
        "with a 2500 ms check: first text {first_text:.0} ms from spawn, marks {:.0} ms after it",
        painted_at - first_at
    );
    assert!(
        painted_at - first_at >= 2400.0,
        "the marks did not wait for the check"
    );
    assert!(
        first_text < 2000.0,
        "first text waited for the check: {first_text:.0} ms"
    );
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_turned_away_chapter_never_paints_and_the_last_one_does() {
    let book = spelling_book();
    let mut command = Command::new(jadeslip_e2e::e2e_binary());
    command
        .arg(&book)
        .env("JADESLIP_E2E_SPELL_DELAY_MS", "400");
    let mut app = App::spawn(command, &[]);
    ready(&mut app);
    app.js(&format!("window.__s.go({})", spelling::LINES));
    std::thread::sleep(Duration::from_millis(30));
    app.js(&format!("window.__s.go({})", spelling::BARE));
    wait_words(&mut app, &planted_in(spelling::BARE));
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        words(&mut app),
        ["acheive"],
        "an older chapter painted late"
    );
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_book_that_says_it_is_french_gets_no_marks() {
    let path = unique_dir("french").join("french.epub");
    spelling::write_french_book(&path).expect("book");
    let mut app = open(&path);
    wait_until(&mut app, "the chapter was checked", |app| {
        app.js("window.__s.painted().length").as_u64() == Some(1)
    });
    assert_eq!(words(&mut app), Vec::<String>::new());
    enter_edit(&mut app);
    assert_eq!(app.js("window.__s.caretAfter('Le')"), json!(true));
    app.type_text(" qiuckzz ");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(words(&mut app), Vec::<String>::new());
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn marks_stay_when_edit_begins_and_read_shows_no_menu() {
    let book = spelling_book();
    let mut app = open(&book);
    wait_words(&mut app, &prose_marks());
    assert_eq!(
        app.js("window.__s.contextMenuOn('recieve')"),
        json!(false),
        "Read must leave the right click alone"
    );
    assert_eq!(app.js("window.__s.menu().open"), json!(false));
    enter_edit(&mut app);
    assert_eq!(words(&mut app).len(), prose_marks().len());
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "read mode", |app| {
        app.js("window.__s.mode()") == json!("read")
    });
    assert_eq!(words(&mut app).len(), prose_marks().len());
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_right_click_fix_clears_the_mark_saves_and_offers_the_other_copies() {
    let book = spelling_book();
    let before = sha(&book);
    let mut app = open(&book);
    wait_words(&mut app, &prose_marks());
    enter_edit(&mut app);
    let menu = open_menu_on(&mut app, "definately");
    let items: Vec<String> = serde_json::from_value(menu["items"].clone()).expect("items");
    assert!(items.iter().any(|item| item == "definitely"), "{items:?}");
    assert_eq!(items.last().map(String::as_str), Some("Add to dictionary"));
    assert_eq!(sha(&book), before, "opening the menu wrote the book");
    click_item(&mut app, "definitely");
    wait_until(&mut app, "the menu closes", |app| {
        app.js("window.__s.menu().open") == json!(false)
    });
    let text = app.js("window.__s.text()");
    let text = text.as_str().expect("text");
    assert!(text.contains("It was definitely a ship"));
    assert!(!text.contains("definately"));
    assert!(!words(&mut app).iter().any(|word| word == "definately"));
    wait_until(&mut app, "the edit counts as unsaved", |app| {
        app.js("window.__s.unsaved()") == json!(true)
    });
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "the fix everywhere offer", |app| {
        app.js("window.__s.offerReady()") == json!(true)
    });
    let offer = app.js("window.__s.offer()");
    let offer = offer.as_str().expect("offer").to_owned();
    println!("offer text after a spelling fix: {offer}");
    assert!(offer.contains('2'), "the two other copies: {offer}");
    assert_ne!(sha(&book), before, "the fix reached the book");
    let mut wanted = prose_marks();
    wanted.retain(|word| *word != "definately");
    wait_words(&mut app, &wanted);
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn escape_closes_the_menu_and_stays_in_edit_and_undo_brings_a_fix_back() {
    let book = spelling_book();
    let mut app = open(&book);
    wait_words(&mut app, &prose_marks());
    enter_edit(&mut app);
    open_menu_on(&mut app, "recieve");
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "the menu closes", |app| {
        app.js("window.__s.menu().open") == json!(false)
    });
    assert_eq!(app.js("window.__s.mode()"), json!("edit"));
    open_menu_on(&mut app, "recieve");
    click_item(&mut app, "receive");
    wait_until(&mut app, "the mark clears", |app| {
        !words(app).iter().any(|word| word == "recieve")
    });
    app.key_with(KEY_Z, MASK_CTRL);
    wait_until(&mut app, "undo restores the word and its mark", |app| {
        words(app).iter().any(|word| word == "recieve")
    });
    assert!(
        app.js("window.__s.text()")
            .as_str()
            .is_some_and(|text| text.contains("recieve"))
    );
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn add_to_dictionary_clears_every_copy_and_survives_a_restart_without_writing_the_book() {
    let book = spelling_book();
    let before = sha(&book);
    let data = unique_dir("data");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    go(&mut app, spelling::AGAIN);
    wait_words(&mut app, &["definately", "definately"]);
    enter_edit(&mut app);
    open_menu_on(&mut app, "definately");
    click_item(&mut app, "Add to dictionary");
    wait_words(&mut app, &[]);
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "read mode", |app| {
        app.js("window.__s.mode()") == json!("read")
    });
    let list = fs::read_to_string(data.join("spelling-words.txt")).expect("word list");
    assert_eq!(list, "definately\n");
    app.close();
    assert_eq!(sha(&book), before, "adding a word wrote the book");
    let mut again = App::launch_with_data(&book, &data);
    ready(&mut again);
    go(&mut again, spelling::AGAIN);
    assert_eq!(words(&mut again), Vec::<String>::new());
    again.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_new_typo_is_marked_once_typing_settles_and_correcting_it_clears_the_mark() {
    let book = spelling_book();
    let mut app = open(&book);
    wait_words(&mut app, &prose_marks());
    enter_edit(&mut app);
    assert_eq!(app.js("window.__s.caretAfter('The lamp')"), json!(false));
    assert_eq!(app.js("window.__s.caretAfter('storm had')"), json!(true));
    app.type_text(" qiuckly");
    let mut wanted = prose_marks();
    wanted.push("qiuckly");
    wait_words(&mut app, &wanted);
    app.js("window.__s.typeAt('qiuckly', 'quickly')");
    wanted.pop();
    wait_words(&mut app, &wanted);
    app.js("window.__s.typeAt('Teh', 'The')");
    wanted.retain(|word| *word != "Teh");
    wait_words(&mut app, &wanted);
    assert_eq!(app.js("window.__s.unsaved()"), json!(true));
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_word_typed_inside_code_stays_unmarked() {
    let book = spelling_book();
    let mut app = open(&book);
    go(&mut app, spelling::TAGGED);
    assert_eq!(words(&mut app), Vec::<String>::new());
    enter_edit(&mut app);
    assert_eq!(app.js("window.__s.caretAfter('zqxwvut')"), json!(true));
    app.type_text("qiuckz");
    wait_until(&mut app, "the typing lands inside the code", |app| {
        app.js("window.__s.text()")
            .as_str()
            .is_some_and(|text| text.contains("zqxwvutqiuckz"))
    });
    assert_eq!(app.js("window.__s.caretAfter('password')"), json!(true));
    app.type_text(" qiuckly");
    wait_words(&mut app, &["qiuckly"]);
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn an_old_revision_gets_no_marks() {
    let book = spelling_book();
    let mut app = open(&book);
    wait_words(&mut app, &prose_marks());
    assert_eq!(
        app.js("window.__s.oldRevision()"),
        json!({"fresh": "ranges", "old": "stale"})
    );
    app.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_late_reply_does_not_repaint_an_edited_block() {
    let book = spelling_book();
    let mut command = Command::new(jadeslip_e2e::e2e_binary());
    command
        .arg(&book)
        .env("JADESLIP_E2E_SPELL_DELAY_MS", "1200");
    let mut app = App::spawn(command, &[]);
    ready(&mut app);
    enter_edit(&mut app);
    assert_eq!(
        app.js("window.__s.replaceText('recieve', 'receive')"),
        json!(true)
    );
    wait_until(&mut app, "the delayed reply arrives", |app| {
        app.js("window.__s.painted().length").as_u64() == Some(1)
    });
    std::thread::sleep(Duration::from_millis(500));
    let now = words(&mut app);
    assert!(!now.iter().any(|word| word == "receive"), "{now:?}");
    assert!(!now.iter().any(|word| word == "recieve"), "{now:?}");
    assert!(now.iter().any(|word| word == "definately"), "{now:?}");
    app.close();
}

fn click_id(app: &mut App, id: &str) {
    let at = app.js(&format!(
        "(()=>{{const box=document.getElementById({}).getBoundingClientRect();return {{x:box.left+box.width/2,y:box.top+box.height/2}}}})()",
        json!(id)
    ));
    let (x, y) = point(&at);
    app.click(x, y);
}

fn on_shelf(app: &mut App) -> bool {
    app.js("!document.getElementById('library').hidden") == json!(true)
}

#[test]
#[ignore = "real app on a headless output"]
fn marks_come_on_mount_from_the_shelf_after_a_chapter_switch_and_after_a_revert() {
    let book = spelling_book();
    let data = unique_dir("shelf").join("data");
    let mut app = App::launch_with_data_opt(None, &data);
    ready(&mut app);
    assert!(on_shelf(&mut app), "the app starts on the shelf");
    app.op(&json!({"op": "drop_paths", "paths": [book]}));
    wait_until(&mut app, "the book on the shelf", |app| {
        app.js("!!document.getElementById('hero-go')") == json!(true)
    });
    click_id(&mut app, "hero-go");
    wait_words(&mut app, &prose_marks());
    go(&mut app, spelling::LINES);
    wait_words(&mut app, &planted_in(spelling::LINES));
    go(&mut app, spelling::PROSE);
    wait_words(&mut app, &prose_marks());
    enter_edit(&mut app);
    open_menu_on(&mut app, "recieve");
    click_item(&mut app, "receive");
    let mut fixed = prose_marks();
    fixed.retain(|word| *word != "recieve");
    wait_words(&mut app, &fixed);
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "the fix is saved", |app| {
        app.js("document.getElementById('toast').textContent")
            .as_str()
            .is_some_and(|toast| toast.contains("Saved"))
    });
    enter_edit(&mut app);
    click_id(&mut app, "tool-revert");
    wait_until(&mut app, "the revert dialog", |app| {
        app.js("!document.getElementById('revert-dialog').hidden") == json!(true)
    });
    app.js("window.__s.clearMarks()");
    click_id(&mut app, "revert-chapter");
    wait_until(&mut app, "the reverted chapter was checked", |app| {
        app.js("window.__s.painted().length").as_u64() != Some(0)
    });
    wait_words(&mut app, &prose_marks());
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "read mode", |app| {
        app.js("window.__s.mode()") == json!("read")
    });
    click_id(&mut app, "go-library");
    wait_until(&mut app, "the shelf", on_shelf);
    assert_eq!(words(&mut app), Vec::<String>::new(), "the closed book's marks stay registered");
    click_id(&mut app, "hero-go");
    wait_words(&mut app, &prose_marks());
    app.close();
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "timing report"]
fn report_first_text_before_and_after() {
    let Some(before) = std::env::var_os("SS_BEFORE_BINARY").map(PathBuf::from) else {
        println!("set SS_BEFORE_BINARY to a build from before spelling");
        return;
    };
    let book = generated_book("large-h1-p");
    let after = jadeslip_e2e::e2e_binary();
    let mut before_ms = Vec::new();
    let mut after_ms = Vec::new();
    for _ in 0..9 {
        for (binary, out) in [(&before, &mut before_ms), (&after, &mut after_ms)] {
            let mut command = Command::new(binary);
            command.arg(&book);
            let mut app = App::spawn(command, &[]);
            let shown = app.wait_first_text();
            out.push(shown - app.spawned_at_ms);
            app.close();
        }
    }
    let spread = |values: &[f64]| (values[0], values[values.len() - 1]);
    let before_median = median(&mut before_ms);
    let after_median = median(&mut after_ms);
    println!(
        "first text, 9 runs each alternating, large-h1-p: before spelling median {before_median:.0} ms range {:?}, after median {after_median:.0} ms range {:?}",
        spread(&before_ms),
        spread(&after_ms)
    );
}

#[test]
#[ignore = "timing report"]
fn report_paint_cost() {
    let worst_dir = unique_dir("worst");
    let worst = worst_dir.join("worst.epub");
    spelling::write_worst_chapter(&worst, 6_000).expect("book");
    let rows: Vec<(&str, PathBuf, u32)> = vec![
        (
            "large-h1-p, about 3000 words",
            generated_book("large-h1-p"),
            3,
        ),
        ("6000 unknown words, one block", worst, 0),
    ];
    for (label, book, ix) in rows {
        let mut app = open(&book);
        let mut totals = Vec::new();
        let mut marks = 0;
        for _ in 0..7 {
            app.js("(window.__s.clearMarks(), window.__s.go(1))");
            std::thread::sleep(Duration::from_millis(50));
            go(&mut app, ix);
            let detail = app.js("window.__s.painted()[0]");
            totals.push(detail["ms"].as_f64().expect("ms"));
            marks = detail["marks"].as_u64().expect("marks");
        }
        let frames = app
            .js("(async()=>{const before=performance.now();await new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(done)));return performance.now()-before})()")
            .as_f64()
            .expect("frames");
        println!(
            "{label}: {marks} marks, ask to painted median {:.1} ms, max {:.1} ms, two frames after {frames:.1} ms, 7 runs",
            median(&mut totals),
            totals[totals.len() - 1]
        );
        app.close();
    }
}

#[test]
#[ignore = "real books, counts only"]
fn report_real_book_marks_counts_only() {
    for (at, book) in real_books().iter().enumerate() {
        let mut app = open(book);
        let linear = app
            .js("Number(document.getElementById('where-title').title.replace(/,/g, '').match(/of (\\d+)/)?.[1] ?? 1)")
            .as_u64()
            .unwrap_or(1) as u32;
        let (mut from_speller, mut painted, mut sampled) = (0, 0, 0);
        let mut times = Vec::new();
        for step in 0..20 {
            go(&mut app, step * linear / 20);
            let detail = app.js("window.__s.painted()[0]");
            from_speller += detail["marks"].as_u64().unwrap_or(0);
            times.push(detail["ms"].as_f64().unwrap_or(0.0));
            painted += words(&mut app).len();
            sampled += 1;
        }
        println!(
            "real book {at}: {linear} chapters, {sampled} sampled, marks from the speller {from_speller}, painted after the code and language filter {painted}, ask to painted median {:.0} ms, max {:.0} ms",
            median(&mut times),
            times[times.len() - 1],
        );
        app.close();
    }
}

fn open_comfort(app: &mut App) {
    if app.js("document.getElementById('comfort').hidden") == json!(true) {
        click_id(app, "toggle-comfort");
    }
    wait_until(app, "the text panel opens", |app| {
        app.js("document.getElementById('comfort').hidden") == json!(false)
    });
    click_id(app, "comfort-tab-spelling");
    wait_until(app, "the spelling tab shows", |app| {
        app.js("document.getElementById('comfort-spelling').hidden") == json!(false)
    });
}

fn close_comfort(app: &mut App) {
    if app.js("document.getElementById('comfort').hidden") == json!(false) {
        click_id(app, "toggle-comfort");
    }
    wait_until(app, "the text panel closes", |app| {
        app.js("document.getElementById('comfort').hidden") == json!(true)
    });
    std::thread::sleep(Duration::from_millis(600));
}

fn open_spelling_prefs(app: &mut App) {
    click_id(app, "reader-more");
    wait_until(app, "the reader menu opens", |app| {
        app.js("document.getElementById('open-prefs').getClientRects().length > 0") == json!(true)
    });
    click_id(app, "open-prefs");
    wait_until(app, "Preferences opens", |app| {
        app.js("document.getElementById('prefs-dialog').hidden") == json!(false)
    });
    click_id(app, "prefs-nav-spelling");
    wait_until(app, "the spelling section shows", |app| {
        app.js("document.getElementById('prefs-section-spelling').hidden") == json!(false)
    });
}

fn close_prefs(app: &mut App) {
    click_id(app, "prefs-close");
    wait_until(app, "Preferences closes", |app| {
        app.js("document.getElementById('prefs-dialog').hidden") == json!(true)
    });
}

fn show_chapter(app: &mut App, ordinal: u32) {
    app.js(&format!("window.__s.go({ordinal})"));
    wait_until(app, "the chapter is shown", |app| {
        app.js("window.__s.shown()") == json!(ordinal)
    });
}

fn run_command(app: &mut App, label: &str) {
    click_id(app, "open-palette");
    wait_until(app, "the palette opens", |app| {
        app.js("document.getElementById('palette').hidden") == json!(false)
    });
    app.type_text(label);
    std::thread::sleep(Duration::from_millis(60));
    app.key(KEY_ENTER);
    wait_until(app, "the palette closes", |app| {
        app.js("document.getElementById('palette').hidden") == json!(true)
    });
}

fn wait_saved_spelling(app: &mut App, data: &Path, wanted: bool) {
    let path = data.join("prefs.json");
    wait_until(app, "the switch is saved", |_| {
        fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|prefs| prefs["spelling"] == json!(wanted))
    });
}

#[test]
#[ignore = "real app on a headless output"]
fn the_switch_clears_marks_stops_checking_and_is_remembered_after_a_restart() {
    let book = spelling_book();
    let data = unique_dir("switch").join("data");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    wait_words(&mut app, &prose_marks());
    open_spelling_prefs(&mut app);
    assert_eq!(app.js("window.__s.switchOn()"), json!(true));
    click_id(&mut app, "prefs-spelling-off");
    wait_until(&mut app, "the marks are gone", |app| {
        app.js("window.__s.has()") == json!(false)
    });
    wait_saved_spelling(&mut app, &data, false);
    close_prefs(&mut app);
    app.js("window.__s.clearMarks()");
    show_chapter(&mut app, spelling::AGAIN);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        app.js("window.__s.painted().length"),
        json!(0),
        "a chapter was checked while off"
    );
    assert_eq!(words(&mut app), Vec::<String>::new());
    app.close();

    let mut again = App::launch_with_data(&book, &data);
    ready(&mut again);
    show_chapter(&mut again, spelling::AGAIN);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        again.js("window.__s.painted().length"),
        json!(0),
        "off was forgotten"
    );
    assert_eq!(words(&mut again), Vec::<String>::new());
    open_spelling_prefs(&mut again);
    assert_eq!(again.js("window.__s.switchOn()"), json!(false));
    click_id(&mut again, "prefs-spelling-on");
    wait_words(&mut again, &["definately", "definately"]);
    close_prefs(&mut again);
    run_command(&mut again, "Spelling marks on or off");
    wait_until(&mut again, "the palette turns marks off", |app| {
        app.js("window.__s.has()") == json!(false)
    });
    run_command(&mut again, "Spelling marks on or off");
    wait_words(&mut again, &["definately", "definately"]);
    wait_saved_spelling(&mut again, &data, true);
    again.close();
}

fn french_book() -> PathBuf {
    let path = unique_dir("french-dict").join("french.epub");
    spelling::write_french_book(&path).expect("book");
    path
}

const LEFT_OUT: &str = "lanterne";

fn install_french(data: &Path) {
    let dir = data.join("dictionaries");
    fs::create_dir_all(&dir).expect("dictionaries");
    let mut known: Vec<String> = spelling::FRENCH
        .split(|ch: char| !ch.is_alphabetic())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .filter(|word| word != LEFT_OUT)
        .collect();
    known.extend(["phare".to_owned()]);
    known.sort();
    known.dedup();
    fs::write(dir.join("fr.aff"), "SET UTF-8\nTRY esaitnrul\n").expect("aff");
    fs::write(
        dir.join("fr.dic"),
        format!("{}\n{}\n", known.len(), known.join("\n")),
    )
    .expect("dic");
}

fn choose_language(app: &mut App, label: &str) {
    open_comfort(app);
    wait_until(app, "the language choices show", |app| {
        app.js("window.__s.languages()") != Value::Null
    });
    let at = app.js(&format!("window.__s.languageButton({})", json!(label)));
    let (x, y) = point(&at);
    app.click(x, y);
    wait_until(app, "the choice is shown as chosen", |app| {
        app.js("window.__s.languages()?.chosen") == json!(label)
    });
}

fn many_marks(app: &mut App) -> bool {
    let now = words(app);
    now.len() > 10 && now.iter().any(|word| word == "gardien")
}

#[test]
#[ignore = "real app on a headless output"]
fn a_book_in_another_language_is_checked_with_its_installed_dictionary_and_can_be_overridden() {
    let book = french_book();
    let data = unique_dir("french-data").join("data");
    install_french(&data);
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    wait_words(&mut app, &[LEFT_OUT]);
    open_comfort(&mut app);
    wait_until(&mut app, "the language choices show", |app| {
        app.js("window.__s.languages()") != Value::Null
    });
    let shown = app.js("window.__s.languages()");
    assert_eq!(shown["labels"], json!(["Auto", "English", "French"]));
    assert_eq!(shown["chosen"], json!("Auto"));
    assert_eq!(shown["hint"], json!("Auto checks this book in French."));
    close_comfort(&mut app);

    choose_language(&mut app, "English");
    wait_until(&mut app, "English marks the French words", many_marks);
    app.close();

    let choices: Vec<PathBuf> = fs::read_dir(data.join("books"))
        .expect("books")
        .filter_map(|entry| Some(entry.ok()?.path().join("spelling.json")))
        .filter(|path| path.is_file())
        .collect();
    assert_eq!(choices.len(), 1, "one book holds the choice");

    let mut again = App::launch_with_data(&book, &data);
    ready(&mut again);
    wait_until(
        &mut again,
        "the English choice survives a restart",
        many_marks,
    );
    choose_language(&mut again, "Auto");
    wait_words(&mut again, &[LEFT_OUT]);
    close_comfort(&mut again);
    enter_edit(&mut again);
    assert_eq!(again.js("window.__s.caretAfter('Le')"), json!(true));
    again.type_text(" qiuckzz ");
    wait_words(&mut again, &[LEFT_OUT, "qiuckzz"]);
    again.close();
}

#[test]
#[ignore = "real app on a headless output"]
fn a_broken_dictionary_shows_a_toast_and_the_book_falls_back_to_english() {
    let book = spelling_book();
    let data = unique_dir("broken").join("data");
    let dir = data.join("dictionaries");
    fs::create_dir_all(&dir).expect("dictionaries");
    fs::write(dir.join("en.aff"), "FLAG nonsense\n").expect("aff");
    fs::write(dir.join("en.dic"), "1\nword\n").expect("dic");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    wait_until(&mut app, "the toast names the broken dictionary", |app| {
        let toast = app.js("window.__s.toast()");
        toast
            .as_str()
            .is_some_and(|text| text.contains("dictionary") && text.contains("English instead"))
    });
    wait_words(&mut app, &prose_marks());
    app.close();
}
