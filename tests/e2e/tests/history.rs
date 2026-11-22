use jadeslip_e2e::{App, KEY_ESCAPE, generated_book, repo_root, scratch_dir};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const KEY_E: u32 = 0x65;
const KEY_ENTER: u32 = 0xff0d;

const INSTALL: &str = r"(()=>{
  const element = (id) => document.getElementById(id);
  const shown = (node) => !!node && !node.hidden && node.getBoundingClientRect().height > 0;
  const spot = (node) => {
    if (!shown(node)) return null;
    const rect = node.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  };
  window.__spot = (id) => spot(element(id));
  window.__view = () => ({
    mode: element('mode-edit').classList.contains('on') ? 'edit' : 'read',
    where: element('where-title').title,
    toast: element('toast').hidden ? '' : element('toast').textContent,
    text: element('book-host').shadowRoot?.querySelector('.ss-body')?.textContent ?? '',
    inReader: !element('reader').hidden,
  });
  window.__history = () => ({
    open: !element('history-dialog').hidden,
    title: element('history-title').textContent,
    rows: [...document.querySelectorAll('#history-list .history-row')].map((row) => ({
      what: row.querySelector('.history-what').textContent,
      named: row.querySelector('.history-what').classList.contains('named'),
      chapters: row.querySelector('.history-chapters').textContent,
      at: spot(row),
    })),
    pointText: shown(element('history-point')) ? element('history-point-text').textContent : '',
    changes: shown(element('history-point')) ? [...document.querySelectorAll('#history-changes .history-change')].map((row) => ({
      entry: row.dataset.entry,
      label: row.querySelector('span').textContent,
      compare: spot(row.querySelector('.history-compare')),
      restore: spot(row.querySelector('.history-restore')),
    })) : [],
    diffShown: shown(element('history-diff')),
    added: [...document.querySelectorAll('#history-diff ins')].map((node) => node.textContent),
    removed: [...document.querySelectorAll('#history-diff del')].map((node) => node.textContent),
    whole: spot(element('history-whole')),
  });
})()";

fn view(app: &mut App) -> Value {
    app.js("window.__view()")
}

fn panel(app: &mut App) -> Value {
    app.js("window.__history()")
}

fn click_spot(app: &mut App, spot: &Value) {
    assert!(spot.is_object(), "the control is not on screen");
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn click_id(app: &mut App, id: &str) {
    let spot = app.js(&format!("window.__spot('{id}')"));
    click_spot(app, &spot);
}

fn wait_until(app: &mut App, what: &str, mut ready: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready(app) {
        assert!(
            Instant::now() < deadline,
            "{what} never happened: {}",
            panel(app)
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn wait_toast(app: &mut App, part: &str) {
    wait_until(app, &format!("toast {part}"), |app| {
        view(app)["toast"].as_str().unwrap_or("").contains(part)
    });
}

fn wait_mode(app: &mut App, mode: &str) {
    wait_until(app, &format!("mode {mode}"), |app| {
        view(app)["mode"] == json!(mode)
    });
}

fn dir_for(label: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("history-{label}-{}", std::process::id()));
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

fn assert_valid(path: &Path) {
    let done = Command::new("python3")
        .arg("-I")
        .arg(repo_root().join("tests/epub_structure.py"))
        .arg(path)
        .output()
        .unwrap();
    let found: Value = serde_json::from_slice(&done.stdout).unwrap();
    assert_eq!(found["problems"], json!([]), "{found}");
    assert_eq!(found["dangling_links"], json!(0));
}

fn launch(dir: &Path) -> (App, PathBuf, PathBuf) {
    let path = writable_copy(&generated_book("toc-nav-only"), &dir.join("book.epub"));
    let original = dir.join("original.epub");
    fs::copy(&path, &original).unwrap();
    let mut app = App::launch_with_data_opt(Some(&path), &dir.join("data"));
    app.wait_first_text();
    app.js(INSTALL);
    wait_until(&mut app, "the reader", |app| {
        let shown = view(app);
        shown["inReader"] == json!(true) && !shown["text"].as_str().unwrap_or("").is_empty()
    });
    (app, path, original)
}

fn edit_chapter(app: &mut App, ix: u32, text: &str) {
    app.js(&format!(
        "(async()=>{{await (await import('/main.js')).goTo({ix})}})()"
    ));
    let number = ix + 1;
    wait_until(app, &format!("chapter {number}"), |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with(&format!("Position {number} "))
    });
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

fn open_history(app: &mut App) {
    click_id(app, "open-palette");
    wait_until(app, "the palette", |app| {
        app.js("!document.getElementById('palette').hidden") == json!(true)
    });
    app.js("(()=>{const input=document.getElementById('palette-input');input.value='History and snapshots';input.dispatchEvent(new Event('input'))})()");
    app.key(KEY_ENTER);
    wait_until(app, "the history panel", |app| {
        panel(app)["open"] == json!(true)
    });
}

fn rows(app: &mut App) -> Vec<Value> {
    panel(app)["rows"].as_array().unwrap().clone()
}

fn open_oldest_point(app: &mut App) -> Vec<Value> {
    let oldest = rows(app).last().unwrap()["at"].clone();
    click_spot(app, &oldest);
    wait_until(app, "the point's changes", |app| {
        !panel(app)["pointText"].as_str().unwrap_or("").is_empty()
    });
    panel(app)["changes"].as_array().unwrap().clone()
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn saves_and_a_named_snapshot_show_in_history_with_a_chapter_diff() {
    let dir = dir_for("list");
    let (mut app, _path, _original) = launch(&dir);

    edit_chapter(&mut app, 2, "QQQONE");
    open_history(&mut app);
    let listed = rows(&mut app);
    assert_eq!(listed.len(), 1, "one save, one row: {listed:?}");
    assert_eq!(listed[0]["what"], json!("Saved"));
    assert!(
        !listed[0]["chapters"].as_str().unwrap().is_empty(),
        "the save names the chapter it changed"
    );

    app.js("document.getElementById('history-name').value='before rewrite'");
    click_id(&mut app, "history-take");
    wait_toast(&mut app, "Snapshot taken.");
    wait_until(&mut app, "the snapshot row", |app| rows(app).len() == 2);
    let listed = rows(&mut app);
    assert_eq!(listed[0]["what"], json!("before rewrite"));
    assert_eq!(listed[0]["named"], json!(true));
    click_id(&mut app, "history-close");
    wait_until(&mut app, "the panel to close", |app| {
        panel(app)["open"] == json!(false)
    });

    edit_chapter(&mut app, 2, "QQQTWO");
    open_history(&mut app);
    let whats: Vec<Value> = rows(&mut app)
        .iter()
        .map(|row| row["what"].clone())
        .collect();
    assert_eq!(
        whats,
        [json!("Saved"), json!("before rewrite"), json!("Saved")]
    );

    let changes = open_oldest_point(&mut app);
    assert_eq!(changes.len(), 1, "{changes:?}");
    click_spot(&mut app, &changes[0]["compare"]);
    wait_until(&mut app, "the diff", |app| {
        panel(app)["diffShown"] == json!(true)
    });
    let shown = panel(&mut app);
    let added = shown["added"].to_string();
    assert!(
        added.contains("QQQONE") && added.contains("QQQTWO"),
        "{added}"
    );
    assert!(
        !shown["removed"].to_string().contains("QQQ"),
        "the old side has no edits: {}",
        shown["removed"]
    );
    assert!(shown["title"].as_str().unwrap().ends_with("then and now"));
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn restoring_a_chapter_and_the_whole_book_returns_exact_bytes_and_is_recorded() {
    let dir = dir_for("restore");
    let (mut app, path, original) = launch(&dir);

    edit_chapter(&mut app, 2, "QQQONE");
    edit_chapter(&mut app, 3, "QQQTWO");
    assert_eq!(differing_entries(&path, &original).len(), 2);

    open_history(&mut app);
    let changes = open_oldest_point(&mut app);
    assert_eq!(changes.len(), 2, "{changes:?}");
    let first = changes[0].clone();
    click_spot(&mut app, &first["restore"]);
    wait_toast(&mut app, "Chapter restored to that point.");
    let left = differing_entries(&path, &original);
    assert_eq!(left.len(), 1, "only the other chapter differs: {left:?}");
    assert!(!left.contains(&first["entry"].as_str().unwrap().to_owned()));

    open_history(&mut app);
    let listed = rows(&mut app);
    assert!(
        listed[0]["what"]
            .as_str()
            .unwrap()
            .starts_with("Restored to"),
        "the restore is itself recorded: {listed:?}"
    );
    open_oldest_point(&mut app);
    let whole = panel(&mut app)["whole"].clone();
    click_spot(&mut app, &whole);
    wait_toast(&mut app, "The whole book is back to that point.");
    assert_eq!(
        differing_entries(&path, &original),
        Vec::<String>::new(),
        "every file is back byte for byte"
    );
    wait_until(&mut app, "the page without the edits", |app| {
        let text = view(app)["text"].as_str().unwrap_or("").to_owned();
        !text.is_empty() && !text.contains("QQQ")
    });

    open_history(&mut app);
    let listed = rows(&mut app);
    assert_eq!(listed.len(), 4, "{listed:?}");
    assert!(
        listed[0]["what"]
            .as_str()
            .unwrap()
            .starts_with("Restored to")
    );
    app.close();
    assert_valid(&path);
    let _ = fs::remove_dir_all(&dir);
}
