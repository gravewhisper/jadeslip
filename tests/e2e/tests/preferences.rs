use jadeslip_e2e::{
    App, KEY_ENTER, KEY_ESCAPE, MASK_CTRL, MASK_SHIFT, generated_book, scratch_dir,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const KEY_E: u32 = 0x65;
const KEY_COMMA: u32 = 0x2c;
const KEY_A_UPPER: u32 = 0x41;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const BOOK: &str = "toc-nav-only";

const INSTALL: &str = r"(()=>{
  const element = (id) => document.getElementById(id);
  const shown = (node) => !!node && !node.hidden && node.getBoundingClientRect().height > 0;
  const spot = (node) => {
    if (!shown(node)) return null;
    node.scrollIntoView({ block: 'nearest' });
    const rect = node.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  };
  const text = (id) => element(id).textContent;
  const on = (id) => element(id).classList.contains('on');
  window.__spot = (id) => spot(element(id));
  window.__p = () => ({
    open: !element('prefs-dialog').hidden,
    section: ['general', 'reading', 'spelling', 'storage', 'shortcuts'].find((name) => !element(`prefs-section-${name}`).hidden) ?? null,
    focus: document.activeElement?.id ?? '',
    focusInside: element('prefs-dialog').contains(document.activeElement),
    comfort: !element('comfort').hidden,
    palette: !element('palette').hidden,
    shelf: !element('library').hidden,
    mode: element('mode-edit').classList.contains('on') ? 'edit' : 'read',
    text: element('book-host').shadowRoot?.querySelector('.ss-body')?.textContent ?? '',
    theme: document.querySelector('.app').classList.contains('light') ? 'light' : 'dark',
    paged: document.querySelector('.app').classList.contains('paged'),
    themeLight: on('prefs-theme-light'),
    pagesAuto: on('prefs-pages-auto'),
    keep: ['days7', 'days30', 'days90', 'year', 'forever'].filter((value) => on(`prefs-keep-${value}`)),
    dataDir: text('prefs-data-dir'),
    total: text('prefs-total'),
    cleanResult: text('prefs-clean-result'),
    books: [...document.querySelectorAll('#prefs-books .prow')].map((row) => ({
      key: row.dataset.key,
      title: row.querySelector('.ptitle').textContent,
      cells: [...row.querySelectorAll('.num')].map((cell) => cell.textContent),
      points: row.querySelector('.num[title]').title,
      deleteBackup: Boolean(row.querySelector('[id^=prefs-backup-delete-]')),
    })),
    confirm: element('prefs-confirm').hidden ? null : {
      title: text('prefs-confirm-title'),
      text: text('prefs-confirm-text'),
      ok: text('prefs-confirm-ok'),
    },
    toast: element('toast').hidden ? '' : element('toast').textContent,
  });
  window.__history = () => ({
    open: !element('history-dialog').hidden,
    rows: [...document.querySelectorAll('#history-list .history-row')].map((row) => ({
      what: row.querySelector('.history-what').textContent,
      named: row.querySelector('.history-what').classList.contains('named'),
      at: spot(row),
    })),
    pointText: shown(element('history-point')) ? element('history-point-text').textContent : '',
    changes: shown(element('history-point')) ? [...document.querySelectorAll('#history-changes .history-change')].map((row) => ({
      entry: row.dataset.entry,
      restore: spot(row.querySelector('.history-restore')),
    })) : [],
    whole: spot(element('history-whole')),
  });
  return true;
})()";

fn state(app: &mut App) -> Value {
    app.js("window.__p()")
}

fn history(app: &mut App) -> Value {
    app.js("window.__history()")
}

fn wait_until(app: &mut App, what: &str, mut ready: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready(app) {
        assert!(
            Instant::now() < deadline,
            "{what} never happened: {}",
            state(app)
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn click_spot(app: &mut App, spot: &Value) {
    assert!(
        spot.is_object(),
        "the control is not on screen: {}",
        state(app)
    );
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn click_id(app: &mut App, id: &str) {
    let spot = app.js(&format!("window.__spot('{id}')"));
    assert!(spot.is_object(), "{id} is not on screen: {}", state(app));
    click_spot(app, &spot);
}

fn open_from_menu(app: &mut App) {
    click_id(app, "reader-more");
    wait_until(app, "the reader menu", |app| {
        app.js("window.__spot('open-prefs')").is_object()
    });
    click_id(app, "open-prefs");
}

fn wait_open(app: &mut App, how: &str) {
    wait_until(app, &format!("Preferences opening from {how}"), |app| {
        state(app)["open"] == json!(true)
    });
}

fn escape_closes(app: &mut App, how: &str) -> Value {
    app.key(KEY_ESCAPE);
    wait_until(
        app,
        &format!("Esc closing Preferences opened from {how}"),
        |app| state(app)["open"] == json!(false),
    );
    let after = state(app);
    assert_eq!(after["focusInside"], json!(false), "{how}: {after}");
    after
}

fn wait_toast(app: &mut App, part: &str) {
    wait_until(app, &format!("toast {part}"), |app| {
        state(app)["toast"].as_str().unwrap_or("").contains(part)
    });
}

fn wait_mode(app: &mut App, mode: &str) {
    wait_until(app, &format!("mode {mode}"), |app| {
        state(app)["mode"] == json!(mode)
    });
}

fn dir_for(label: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("prefs-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn writable_copy(source: &Path, path: &Path) -> PathBuf {
    fs::copy(source, path).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(path, perms).unwrap();
    path.to_owned()
}

fn differing_entries(one: &Path, two: &Path) -> Vec<String> {
    let script = "import json,sys,zipfile\na=zipfile.ZipFile(sys.argv[1]);b=zipfile.ZipFile(sys.argv[2])\nnames=sorted(set(a.namelist())|set(b.namelist()))\nout=[n for n in names if n not in a.namelist() or n not in b.namelist() or a.read(n)!=b.read(n)]\nprint(json.dumps(out))";
    let done = Command::new("python3")
        .args(["-I", "-c", script])
        .arg(one)
        .arg(two)
        .output()
        .unwrap();
    serde_json::from_slice(&done.stdout).unwrap()
}

fn ready(app: &mut App) {
    app.wait_first_text();
    app.js(INSTALL);
    wait_until(app, "the reader", |app| {
        let shown = state(app);
        shown["shelf"] == json!(false) && !shown["text"].as_str().unwrap_or("").is_empty()
    });
}

fn launch(book: &Path, data: &Path, clock_ahead_ms: u64) -> App {
    let ahead = clock_ahead_ms.to_string();
    let mut app = App::launch_with_data_env(
        Some(book),
        data,
        &[("JADESLIP_E2E_CLOCK_AHEAD_MS", Path::new(&ahead))],
    );
    ready(&mut app);
    app
}

fn go_to(app: &mut App, ix: u32) {
    app.js(&format!(
        "(async()=>{{await (await import('/main.js')).goTo({ix})}})()"
    ));
    let number = ix + 1;
    wait_until(app, &format!("chapter {number}"), |app| {
        app.js("document.getElementById('where-title').title")
            .as_str()
            .unwrap_or("")
            .starts_with(&format!("Position {number} "))
    });
}

fn edit_chapter(app: &mut App, ix: u32, text: &str) {
    go_to(app, ix);
    app.key(KEY_E);
    wait_mode(app, "edit");
    app.type_text(text);
    app.key(KEY_ESCAPE);
    wait_mode(app, "read");
    wait_toast(app, "Saved");
    wait_until(app, "the save to finish", |app| {
        app.js("document.getElementById('unsaved').hidden") == json!(true)
    });
}

fn palette_run(app: &mut App, label: &str) {
    click_id(app, "open-palette");
    wait_until(app, "the palette", |app| {
        state(app)["palette"] == json!(true)
    });
    app.js(&format!(
        "(()=>{{const input=document.getElementById('palette-input');input.value={};input.dispatchEvent(new Event('input'))}})()",
        json!(label)
    ));
    app.key(KEY_ENTER);
}

fn open_history(app: &mut App) {
    palette_run(app, "History and snapshots");
    wait_until(app, "the history panel", |app| {
        history(app)["open"] == json!(true)
    });
}

fn close_history(app: &mut App) {
    click_id(app, "history-close");
    wait_until(app, "the history panel closing", |app| {
        history(app)["open"] == json!(false)
    });
}

fn history_rows(app: &mut App) -> Vec<Value> {
    history(app)["rows"].as_array().unwrap().clone()
}

fn open_point(app: &mut App, what: &str) {
    let row = history_rows(app)
        .into_iter()
        .find(|row| row["what"] == json!(what))
        .unwrap_or_else(|| panic!("no history row {what}"));
    click_spot(app, &row["at"]);
    wait_until(app, &format!("the point {what}"), |app| {
        !history(app)["pointText"].as_str().unwrap_or("").is_empty()
    });
}

fn take_snapshot(app: &mut App, name: &str) {
    open_history(app);
    let before = history_rows(app).len();
    app.js(&format!(
        "document.getElementById('history-name').value={}",
        json!(name)
    ));
    click_id(app, "history-take");
    wait_toast(app, "Snapshot taken.");
    wait_until(app, "the snapshot row", |app| {
        history_rows(app).len() == before + 1
    });
    close_history(app);
}

fn open_storage(app: &mut App) {
    open_from_menu(app);
    wait_open(app, "the reader menu");
    click_id(app, "prefs-nav-storage");
    wait_until(app, "the storage numbers", |app| {
        let shown = state(app);
        shown["section"] == json!("storage")
            && !shown["total"].as_str().unwrap_or("").is_empty()
            && !shown["books"].as_array().unwrap().is_empty()
    });
}

fn bytes_of(size: &Value) -> f64 {
    let text = size.as_str().unwrap();
    let (number, unit) = text.split_once(' ').unwrap();
    let scale = match unit {
        "B" => 1.0,
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        _ => panic!("size {text}"),
    };
    number.replace(',', "").parse::<f64>().unwrap() * scale
}

fn tree_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    fs::read_dir(path).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| tree_bytes(&entry.path()))
            .sum()
    })
}

fn history_bytes(data: &Path) -> u64 {
    fs::read_dir(data.join("books")).map_or(0, |books| {
        books
            .filter_map(Result::ok)
            .map(|book| tree_bytes(&book.path().join("history")))
            .sum()
    })
}

fn saved_prefs(data: &Path) -> Value {
    fs::read_to_string(data.join("prefs.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn wait_saved(app: &mut App, data: &Path, key: &str, value: &str) {
    wait_until(app, &format!("prefs.json holding {key} {value}"), |_| {
        saved_prefs(data)[key] == json!(value)
    });
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn every_opener_shows_preferences_and_esc_gives_focus_back() {
    let dir = dir_for("openers");
    let book = writable_copy(&generated_book(BOOK), &dir.join("book.epub"));
    let mut app = launch(&book, &dir.join("data"), 0);

    open_from_menu(&mut app);
    wait_open(&mut app, "the reader menu");
    assert_eq!(state(&mut app)["section"], json!("general"));
    let after = escape_closes(&mut app, "the reader menu");
    assert_eq!(
        after["focus"],
        json!("reader-more"),
        "focus goes back to the menu button"
    );

    app.key_with(KEY_COMMA, MASK_CTRL);
    wait_open(&mut app, "Ctrl+, in Read");
    escape_closes(&mut app, "Ctrl+, in Read");
    assert_eq!(state(&mut app)["mode"], json!("read"));

    palette_run(&mut app, "Preferences");
    wait_open(&mut app, "the palette");
    assert_eq!(state(&mut app)["palette"], json!(false));
    escape_closes(&mut app, "the palette");

    app.key_with(KEY_A_UPPER, MASK_CTRL | MASK_SHIFT);
    wait_until(&mut app, "Ctrl+Shift+A opening Aa in Read", |app| {
        state(app)["comfort"] == json!(true)
    });
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "Esc closing Aa", |app| {
        state(app)["comfort"] == json!(false)
    });

    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    let text = state(&mut app)["text"].clone();
    app.key_with(KEY_COMMA, MASK_CTRL);
    wait_open(&mut app, "Ctrl+, in Edit");
    escape_closes(&mut app, "Ctrl+, in Edit");
    let back = state(&mut app);
    assert_eq!(
        back["mode"],
        json!("edit"),
        "Esc closes Preferences, not Edit"
    );
    app.key_with(KEY_A_UPPER, MASK_CTRL | MASK_SHIFT);
    wait_until(&mut app, "Ctrl+Shift+A opening Aa in Edit", |app| {
        state(app)["comfort"] == json!(true)
    });
    std::thread::sleep(Duration::from_millis(200));
    let shown = state(&mut app);
    assert_eq!(shown["text"], text, "Ctrl+Shift+A does not touch the text");
    assert_eq!(shown["mode"], json!("edit"));
    assert_eq!(
        app.js("document.getElementById('unsaved').hidden"),
        json!(true),
        "nothing to save"
    );
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "Esc closing Aa in Edit", |app| {
        state(app)["comfort"] == json!(false)
    });
    app.close();

    let mut shelf = App::launch_with_data_opt(None, &dir.join("data"));
    shelf.wait_first_text();
    shelf.js(INSTALL);
    wait_until(&mut shelf, "the shelf", |app| {
        state(app)["shelf"] == json!(true)
    });
    click_id(&mut shelf, "shelf-prefs");
    wait_open(&mut shelf, "the shelf gear");
    let after = escape_closes(&mut shelf, "the shelf gear");
    assert_eq!(after["focus"], json!("shelf-prefs"));
    assert_eq!(after["shelf"], json!(true));
    shelf.key_with(KEY_COMMA, MASK_CTRL);
    wait_open(&mut shelf, "Ctrl+, on the shelf");
    escape_closes(&mut shelf, "Ctrl+, on the shelf");
    shelf.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn light_theme_and_auto_pages_chosen_in_preferences_apply_on_the_next_launch() {
    let dir = dir_for("persist");
    let book = writable_copy(&generated_book(BOOK), &dir.join("book.epub"));
    let data = dir.join("data");
    let mut app = launch(&book, &data, 0);
    let first = state(&mut app);
    assert_eq!(first["theme"], json!("dark"));
    assert_eq!(first["paged"], json!(false));
    open_from_menu(&mut app);
    wait_open(&mut app, "the reader menu");
    click_id(&mut app, "prefs-theme-light");
    click_id(&mut app, "prefs-pages-auto");
    wait_until(&mut app, "light and paged now", |app| {
        let shown = state(app);
        shown["theme"] == json!("light")
            && shown["paged"] == json!(true)
            && shown["themeLight"] == json!(true)
            && shown["pagesAuto"] == json!(true)
    });
    wait_saved(&mut app, &data, "theme", "light");
    wait_saved(&mut app, &data, "pages", "auto");
    app.close();

    let mut again = launch(&book, &data, 0);
    let shown = state(&mut again);
    assert_eq!(shown["theme"], json!("light"), "{shown}");
    assert_eq!(shown["paged"], json!(true), "{shown}");
    assert_eq!(shown["themeLight"], json!(true));
    assert_eq!(shown["pagesAuto"], json!(true));
    assert_eq!(
        again.js("getComputedStyle(document.body).backgroundColor"),
        "rgb(244, 245, 243)"
    );
    let saved = saved_prefs(&data);
    assert_eq!(saved["theme"], json!("light"), "{saved}");
    assert_eq!(saved["pages"], json!("auto"), "{saved}");
    again.close();
    let _ = fs::remove_dir_all(&dir);
}

fn book_with_history(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let book = writable_copy(&generated_book(BOOK), &dir.join("book.epub"));
    let data = dir.join("data");
    let at_snapshot = dir.join("at-snapshot.epub");
    let mut app = launch(&book, &data, 0);
    edit_chapter(&mut app, 2, "QQQONE");
    fs::copy(&book, &at_snapshot).unwrap();
    take_snapshot(&mut app, "keep me");
    edit_chapter(&mut app, 3, "QQQTWO");
    edit_chapter(&mut app, 2, "QQQTHREE");
    open_history(&mut app);
    let whats: Vec<Value> = history_rows(&mut app)
        .iter()
        .map(|row| row["what"].clone())
        .collect();
    assert_eq!(
        whats,
        [
            json!("Saved"),
            json!("Saved"),
            json!("keep me"),
            json!("Saved")
        ]
    );
    close_history(&mut app);
    app.close();
    (book, data, at_snapshot)
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn storage_lists_the_data_folder_and_the_book_with_its_history_and_backup() {
    let dir = dir_for("storage");
    let (book, data, _) = book_with_history(&dir);
    let mut app = launch(&book, &data, 0);
    click_id(&mut app, "add-to-shelf");
    wait_until(&mut app, "the book on the shelf", |app| {
        app.js("document.getElementById('add-to-shelf').hidden") == json!(true)
    });
    open_storage(&mut app);
    let shown = state(&mut app);
    println!("storage: {shown}");
    assert_eq!(shown["dataDir"], json!(data.display().to_string()));
    assert!(bytes_of(&shown["total"]) > 0.0);
    let books = shown["books"].as_array().unwrap();
    assert_eq!(books.len(), 1, "{books:?}");
    let row = &books[0];
    assert_eq!(row["key"].as_str().unwrap().len(), 32, "{row}");
    assert!(
        data.join("books")
            .join(row["key"].as_str().unwrap())
            .is_dir()
    );
    assert_ne!(row["title"], json!("Not on the shelf"));
    let cells = row["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 5, "{row}");
    let copy = fs::read_dir(data.join("library"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "epub"))
        .unwrap();
    let copied = copy.metadata().unwrap().len() as f64;
    assert!(bytes_of(&cells[0]) > 0.0, "the book file has bytes: {row}");
    assert!(
        (bytes_of(&cells[0]) - copied).abs() <= 500_f64.max(copied * 0.05),
        "the book file is the library copy of {copied} bytes: {row}"
    );
    assert!(bytes_of(&cells[1]) > 0.0, "history has bytes: {row}");
    assert!(bytes_of(&cells[2]) > 0.0, "the backup has bytes: {row}");
    assert!(bytes_of(&cells[4]) > 0.0, "the total has bytes: {row}");
    assert_eq!(row["points"], json!("3 automatic, 1 named"));
    assert_eq!(row["deleteBackup"], json!(true));
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn clean_up_now_drops_old_automatic_points_and_keeps_the_named_one_and_the_newest() {
    let dir = dir_for("clean");
    let (book, data, at_snapshot) = book_with_history(&dir);
    let mut prefs = saved_prefs(&data);
    if !prefs.is_object() {
        prefs = json!({});
    }
    prefs["keepSnapshots"] = json!("days7");
    fs::write(data.join("prefs.json"), prefs.to_string()).unwrap();
    let history_before = history_bytes(&data);

    let mut app = launch(&book, &data, 8 * DAY_MS);
    open_storage(&mut app);
    let before = state(&mut app);
    assert_eq!(before["keep"], json!(["days7"]));
    click_id(&mut app, "prefs-clean-now");
    wait_until(&mut app, "the cleanup result", |app| {
        state(app)["cleanResult"]
            .as_str()
            .unwrap_or("")
            .starts_with("Freed ")
    });
    wait_until(&mut app, "the storage numbers to refresh", |app| {
        state(app)["books"][0]["points"] == json!("1 automatic, 1 named")
    });
    let after = state(&mut app);
    println!("before: {before}\nafter: {after}");
    assert_eq!(
        after["cleanResult"]
            .as_str()
            .unwrap()
            .split(" from ")
            .nth(1),
        Some("1 book")
    );
    assert!(
        bytes_of(&after["total"]) < bytes_of(&before["total"]),
        "the total drops: {} then {}",
        before["total"],
        after["total"]
    );
    let history_after = history_bytes(&data);
    assert!(
        history_after < history_before,
        "{history_before} then {history_after}"
    );
    click_id(&mut app, "prefs-clean-now");
    wait_until(&mut app, "a second cleanup", |app| {
        state(app)["cleanResult"] == json!("Nothing to clean up")
    });
    click_id(&mut app, "prefs-close");
    wait_until(&mut app, "Preferences closing", |app| {
        state(app)["open"] == json!(false)
    });

    open_history(&mut app);
    let whats: Vec<Value> = history_rows(&mut app)
        .iter()
        .map(|row| row["what"].clone())
        .collect();
    assert_eq!(whats, [json!("Saved"), json!("keep me")]);
    open_point(&mut app, "Saved");
    let changes = history(&mut app)["changes"].as_array().unwrap().clone();
    assert_eq!(changes.len(), 1, "{changes:?}");
    click_spot(&mut app, &changes[0]["restore"]);
    wait_toast(&mut app, "Chapter restored to that point.");
    let left = differing_entries(&book, &at_snapshot);
    assert_eq!(
        left.len(),
        1,
        "only the chapter with QQQTWO differs: {left:?}"
    );
    assert_ne!(
        Some(&left[0]),
        changes[0]["entry"].as_str().map(str::to_owned).as_ref()
    );

    open_history(&mut app);
    open_point(&mut app, "keep me");
    let whole = history(&mut app)["whole"].clone();
    click_spot(&mut app, &whole);
    wait_toast(&mut app, "The whole book is back to that point.");
    assert_eq!(
        differing_entries(&book, &at_snapshot),
        Vec::<String>::new(),
        "the book is the one the named snapshot kept"
    );
    wait_until(&mut app, "the page with the first edit only", |app| {
        let text = state(app)["text"].as_str().unwrap_or("").to_owned();
        !text.contains("QQQTWO") && !text.contains("QQQTHREE")
    });
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_shorter_period_asks_with_counts_and_cancel_keeps_the_old_one() {
    let dir = dir_for("shorter");
    let (book, data, _) = book_with_history(&dir);
    let mut prefs = saved_prefs(&data);
    if !prefs.is_object() {
        prefs = json!({});
    }
    prefs["keepSnapshots"] = json!("days90");
    fs::write(data.join("prefs.json"), prefs.to_string()).unwrap();

    let mut app = launch(&book, &data, 8 * DAY_MS);
    open_storage(&mut app);
    assert_eq!(state(&mut app)["keep"], json!(["days90"]));
    click_id(&mut app, "prefs-keep-year");
    wait_until(&mut app, "a longer period without asking", |app| {
        state(app)["keep"] == json!(["year"])
    });
    assert_eq!(state(&mut app)["confirm"], Value::Null);
    wait_saved(&mut app, &data, "keepSnapshots", "year");

    click_id(&mut app, "prefs-keep-days7");
    wait_until(&mut app, "the confirm", |app| {
        !state(app)["confirm"].is_null()
    });
    let asked = state(&mut app)["confirm"].clone();
    println!("asked: {asked}");
    assert_eq!(asked["title"], json!("Remove older history?"));
    assert!(
        asked["text"].as_str().unwrap().starts_with(
            "This removes 2 automatic history points older than 7 days from 1 book and frees "
        ),
        "{asked}"
    );
    assert_eq!(asked["ok"], json!("Remove"));
    assert_eq!(
        app.js("document.activeElement.id"),
        json!("prefs-confirm-cancel"),
        "Cancel has the focus"
    );
    click_id(&mut app, "prefs-confirm-cancel");
    wait_until(&mut app, "the confirm closing", |app| {
        state(app)["confirm"].is_null()
    });
    let shown = state(&mut app);
    assert_eq!(shown["keep"], json!(["year"]), "{shown}");
    assert_eq!(shown["open"], json!(true));
    assert_eq!(saved_prefs(&data)["keepSnapshots"], json!("year"));
    assert_eq!(shown["books"][0]["points"], json!("3 automatic, 1 named"));

    click_id(&mut app, "prefs-keep-days7");
    wait_until(&mut app, "the confirm again", |app| {
        !state(app)["confirm"].is_null()
    });
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "Esc closing only the confirm", |app| {
        state(app)["confirm"].is_null()
    });
    let shown = state(&mut app);
    assert_eq!(shown["open"], json!(true), "Preferences stays open");
    assert_eq!(shown["keep"], json!(["year"]));

    click_id(&mut app, "prefs-keep-days7");
    wait_until(&mut app, "the confirm a third time", |app| {
        !state(app)["confirm"].is_null()
    });
    click_id(&mut app, "prefs-confirm-ok");
    wait_until(&mut app, "the cleanup after Remove", |app| {
        let shown = state(app);
        shown["keep"] == json!(["days7"])
            && shown["cleanResult"]
                .as_str()
                .unwrap_or("")
                .starts_with("Freed ")
            && shown["books"][0]["points"] == json!("1 automatic, 1 named")
    });
    wait_saved(&mut app, &data, "keepSnapshots", "days7");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn deleting_the_original_backup_asks_then_revert_has_no_backup() {
    let dir = dir_for("backup");
    let (book, data, _) = book_with_history(&dir);
    let mut app = launch(&book, &data, 0);
    open_storage(&mut app);
    let row = state(&mut app)["books"][0].clone();
    let key = row["key"].as_str().unwrap().to_owned();
    assert!(bytes_of(&row["cells"][2]) > 0.0, "{row}");
    let backups = data.join("books").join(&key).join("backups");
    assert!(backups.is_dir());

    click_id(&mut app, &format!("prefs-backup-delete-{key}"));
    wait_until(&mut app, "the backup confirm", |app| {
        !state(app)["confirm"].is_null()
    });
    let asked = state(&mut app)["confirm"].clone();
    assert_eq!(asked["title"], json!("Delete the original backup?"));
    assert_eq!(asked["ok"], json!("Delete backup"));
    click_id(&mut app, "prefs-confirm-cancel");
    wait_until(&mut app, "the confirm closing", |app| {
        state(app)["confirm"].is_null()
    });
    assert!(backups.is_dir(), "Cancel keeps the backup");

    click_id(&mut app, &format!("prefs-backup-delete-{key}"));
    wait_until(&mut app, "the backup confirm again", |app| {
        !state(app)["confirm"].is_null()
    });
    click_id(&mut app, "prefs-confirm-ok");
    wait_until(&mut app, "the row without a backup", |app| {
        let row = state(app)["books"][0].clone();
        row["cells"][2] == json!("None") && row["deleteBackup"] == json!(false)
    });
    assert!(!backups.exists());
    let row = state(&mut app)["books"][0].clone();
    assert!(bytes_of(&row["cells"][1]) > 0.0, "history stays: {row}");
    click_id(&mut app, "prefs-close");
    wait_until(&mut app, "Preferences closing", |app| {
        state(app)["open"] == json!(false)
    });

    palette_run(&mut app, "Revert to backup");
    wait_toast(
        &mut app,
        "This book has no original backup. The next edit makes one from the file as it is now.",
    );
    app.close();
    let _ = fs::remove_dir_all(&dir);
}
