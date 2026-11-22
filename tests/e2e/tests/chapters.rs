use serde_json::{Value, json};
use jadeslip_core::{DEFAULT_LIMITS, Reader};
use jadeslip_e2e::{
    App, KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_UP, generated_book, largest_real_book, repo_root,
    scratch_dir,
};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const KEY_E: u32 = 0x65;
const KEY_F2: u32 = 0xffbf;
const MASK_ALT: u32 = 8;

const INSTALL: &str = r"(()=>{
  const element = (id) => document.getElementById(id);
  const box = (node) => {
    const rect = node.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, top: rect.top, height: rect.height };
  };
  window.__box = (id) => box(element(id));
  window.__rows = () => [...document.querySelectorAll('#toc-spacer .row:not([hidden])')]
    .map((row) => ({ entry: Number(row.dataset.entry), t: row.querySelector('.t').textContent, chev: row.querySelector('.chev').textContent, fold: box(row.querySelector('.chev')), ...box(row) }))
    .sort((a, b) => a.entry - b.entry);
  window.__view = () => ({
    mode: element('mode-edit').classList.contains('on') ? 'edit' : 'read',
    where: element('where-title').title,
    title: element('where-title').textContent,
    unsaved: !element('unsaved').hidden,
    dialog: !element('dialog').hidden,
    asking: !element('chapter-dialog').hidden,
    toast: element('toast').hidden ? '' : element('toast').textContent,
    undo: !element('undo-bar').hidden,
    notice: element('notice').hidden ? '' : element('notice').textContent,
    actions: !element('rail-actions').hidden,
    add: !element('add-chapter').hidden,
    crumb: element('crumb').textContent,
    deleted: element('deleted-open').hidden ? '' : element('deleted-open').textContent,
    text: element('book-host').shadowRoot?.querySelector('.ss-body')?.textContent ?? '',
    active: document.activeElement?.id ?? '',
    rename: !element('rail-rename').hidden,
  });
})()";

fn install(app: &mut App) {
    app.js(INSTALL);
}

fn rows(app: &mut App) -> Vec<Value> {
    app.js("window.__rows()").as_array().unwrap().clone()
}

fn titles(app: &mut App) -> Vec<String> {
    rows(app)
        .iter()
        .map(|row| row["t"].as_str().unwrap().to_owned())
        .collect()
}

fn view(app: &mut App) -> Value {
    app.js("window.__view()")
}

fn click_id(app: &mut App, id: &str) {
    let spot = app.js(&format!("window.__box('{id}')"));
    assert!(
        spot["height"].as_f64().unwrap_or(0.0) > 0.0,
        "{id} is not on screen"
    );
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn wait_until(app: &mut App, what: &str, mut ready: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready(app) {
        assert!(
            Instant::now() < deadline,
            "{what} never happened: {}",
            view(app)
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn wait_mode(app: &mut App, mode: &str) {
    wait_until(app, &format!("mode {mode}"), |app| {
        view(app)["mode"] == json!(mode)
    });
}

fn spine_names(path: &Path) -> Vec<String> {
    Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS)
        .unwrap()
        .rows()
        .iter()
        .map(|row| row.href.as_ref().unwrap().as_str().to_owned())
        .collect()
}

fn validator_clean(path: &Path) {
    let output = Command::new("python3")
        .arg("-I")
        .arg(repo_root().join("tests/epub_structure.py"))
        .arg(path)
        .output()
        .unwrap();
    let out: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(out["problems"], json!([]), "validator: {out}");
}

fn new_scratch(label: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("ch-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn set_field(app: &mut App, id: &str, text: &str) {
    app.js(&format!(
        "document.getElementById('{id}').value = {}",
        json!(text)
    ));
}

fn library_books(data: &Path) -> Vec<PathBuf> {
    fs::read_dir(data.join("library"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn create_book(app: &mut App, title: &str) -> PathBuf {
    click_id(app, "shelf-new");
    wait_until(app, "the new book dialog", |app| {
        app.js("!document.getElementById('newbook-dialog').hidden") == json!(true)
    });
    set_field(app, "newbook-name", title);
    set_field(app, "newbook-author", "Some Author");
    set_field(app, "newbook-first", "The Start");
    click_id(app, "newbook-create");
    wait_mode(app, "edit");
    wait_until(app, "the rail row", |app| {
        titles(app) == vec!["The Start".to_owned()]
    });
    let made = library_books(app.data_dir());
    assert_eq!(made.len(), 1, "one new file in the library: {made:?}");
    made[0].clone()
}

fn drag_row(app: &mut App, from: &Value, to: &Value, lower: bool) {
    let x = from["x"].as_f64().unwrap() - 60.0;
    let from_y = from["y"].as_f64().unwrap();
    let to_y = to["top"].as_f64().unwrap()
        + to["height"].as_f64().unwrap() * if lower { 0.8 } else { 0.2 };
    app.op(&json!({"op": "press", "x": x, "y": from_y}));
    let steps = 6;
    for step in 1..=steps {
        let y = from_y + (to_y - from_y) * f64::from(step) / f64::from(steps);
        app.move_pointer(x, y);
        std::thread::sleep(Duration::from_millis(16));
    }
    let line = app.js("!document.getElementById('rail-drop').hidden");
    assert_eq!(line, json!(true), "no drop line showed during the drag");
    app.op(&json!({"op": "release", "x": x, "y": to_y}));
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_book_is_made_written_and_managed_from_an_empty_launch() {
    let dir = new_scratch("flow");
    let data = dir.join("data");
    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    app.assert_on_headless_output();
    install(&mut app);

    let path = create_book(&mut app, "My <Own> Book");
    assert_eq!(view(&mut app)["crumb"], json!("My <Own> Book"));
    assert!(
        path.file_name().unwrap().to_string_lossy().starts_with("My Own Book - "),
        "the book is named for its title: {}",
        path.display()
    );
    validator_clean(&path);
    assert_eq!(
        view(&mut app)["add"],
        json!(true),
        "Add chapter is offered in Edit"
    );

    app.type_text("Hello there");
    wait_until(&mut app, "the unsaved mark", |app| {
        view(app)["unsaved"] == json!(true)
    });
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");
    wait_until(&mut app, "the save", |app| {
        view(app)["unsaved"] == json!(false)
    });
    assert!(
        String::from_utf8_lossy(&chapter_bytes(&path, 0)).contains("Hello there"),
        "the typed text is not in the file"
    );
    assert_eq!(
        view(&mut app)["add"],
        json!(false),
        "Read keeps the rail plain"
    );
    validator_clean(&path);

    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    click_id(&mut app, "add-chapter");
    wait_until(&mut app, "a second chapter", |app| {
        titles(app).len() == 2
            && view(app)["mode"] == json!("edit")
            && view(app)["where"] == json!("Position 2 of 2")
    });
    assert_eq!(spine_names(&path).len(), 2);
    validator_clean(&path);

    app.key(KEY_F2);
    wait_until(&mut app, "the rename field", |app| {
        view(app)["rename"] == json!(true)
    });
    app.js("document.getElementById('rail-rename').select()");
    app.type_text("Middle");
    app.key(KEY_ENTER);
    wait_until(&mut app, "the renamed row", |app| {
        titles(app) == vec!["The Start".to_owned(), "Middle".to_owned()]
    });
    wait_until(&mut app, "the renamed heading", |app| {
        view(app)["text"].as_str().unwrap().contains("Middle")
    });
    validator_clean(&path);

    click_id(&mut app, "add-chapter");
    wait_until(&mut app, "a third chapter", |app| {
        titles(app).len() == 3 && view(app)["where"] == json!("Position 3 of 3")
    });
    wait_mode(&mut app, "edit");
    app.key(KEY_F2);
    wait_until(&mut app, "the rename field", |app| {
        view(app)["rename"] == json!(true)
    });
    app.js("document.getElementById('rail-rename').select()");
    app.type_text("Last");
    app.key(KEY_ENTER);
    wait_until(&mut app, "the third name", |app| titles(app)[2] == "Last");

    let before = spine_names(&path);
    let shown = rows(&mut app);
    drag_row(&mut app, &shown[2], &shown[0], false);
    wait_until(&mut app, "the drag reorder", |app| {
        titles(app)
            == vec![
                "Last".to_owned(),
                "The Start".to_owned(),
                "Middle".to_owned(),
            ]
    });
    assert_eq!(
        spine_names(&path),
        vec![before[2].clone(), before[0].clone(), before[1].clone()],
        "a drag did not reorder the spine"
    );
    validator_clean(&path);
    wait_until(&mut app, "the shown chapter keeps its place", |app| {
        view(app)["where"] == json!("Position 1 of 3")
    });

    app.key_with(KEY_DOWN, MASK_ALT);
    wait_until(&mut app, "the keyboard reorder", |app| {
        titles(app)
            == vec![
                "The Start".to_owned(),
                "Last".to_owned(),
                "Middle".to_owned(),
            ]
    });
    assert_eq!(spine_names(&path)[1], before[2], "Alt+Down did not move it");
    app.key_with(KEY_UP, MASK_ALT);
    wait_until(&mut app, "the reorder back", |app| titles(app)[0] == "Last");
    app.key_with(KEY_DOWN, MASK_ALT);
    wait_until(&mut app, "the reorder again", |app| {
        titles(app)[1] == "Last"
    });
    wait_until(&mut app, "the editor to take keys again", |app| {
        app.js("!!document.getElementById('book-host').shadowRoot.querySelector('[contenteditable]')")
            == json!(true)
    });

    app.type_text("unsaved words");
    wait_until(&mut app, "dirty", |app| view(app)["unsaved"] == json!(true));
    wait_until(&mut app, "every typed key", |app| {
        view(app)["text"].as_str().unwrap_or("").contains("unsaved words")
    });
    click_id(&mut app, "add-chapter");
    wait_until(&mut app, "the unsaved prompt", |app| {
        view(app)["dialog"] == json!(true)
    });
    assert_eq!(
        spine_names(&path).len(),
        3,
        "the op ran before the prompt was answered"
    );
    click_id(&mut app, "dialog-save");
    wait_until(&mut app, "the fourth chapter after the prompt", |app| {
        titles(app).len() == 4 && view(app)["mode"] == json!("edit")
    });
    let reader = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
    let last_row = reader
        .rows()
        .iter()
        .find(|row| row.href.as_ref().unwrap().as_str() == before[2])
        .unwrap()
        .ix;
    let last_body = String::from_utf8_lossy(&chapter_bytes(&path, last_row.0)).into_owned();
    assert!(
        last_body.contains("<h1>Last</h1>"),
        "the caret left the text for the heading"
    );
    assert!(
        last_body.contains("unsaved words"),
        "the prompt's Save lost the words"
    );
    drop(reader);

    let doomed_at = 2;
    let doomed = spine_names(&path)[doomed_at].clone();
    let doomed_bytes = chapter_bytes(&path, doomed_at as u32);
    let order_before = spine_names(&path);
    let entry_y = rows(&mut app)[doomed_at].clone();
    app.move_pointer(
        entry_y["x"].as_f64().unwrap() - 60.0,
        entry_y["y"].as_f64().unwrap(),
    );
    wait_until(&mut app, "the row actions", |app| {
        app.js("!document.getElementById('rail-actions').hidden && document.getElementById('rail-actions').dataset.entry") == json!(doomed_at.to_string())
    });
    click_id(&mut app, "rail-delete");
    wait_until(&mut app, "the delete question", |app| {
        view(app)["asking"] == json!(true)
    });
    assert_eq!(
        spine_names(&path),
        order_before,
        "delete ran before the answer"
    );
    click_id(&mut app, "chapter-dialog-cancel");
    wait_until(&mut app, "the cancel", |app| {
        view(app)["asking"] == json!(false)
    });
    assert_eq!(spine_names(&path), order_before);
    app.move_pointer(
        entry_y["x"].as_f64().unwrap() - 60.0,
        entry_y["y"].as_f64().unwrap() + 1.0,
    );
    click_id(&mut app, "rail-delete");
    wait_until(&mut app, "the delete question", |app| {
        view(app)["asking"] == json!(true)
    });
    click_id(&mut app, "chapter-dialog-ok");
    wait_until(&mut app, "the undo bar", |app| {
        view(app)["undo"] == json!(true)
    });
    assert!(
        !spine_names(&path).contains(&doomed),
        "the chapter is still in the book"
    );
    wait_until(&mut app, "the deleted list", |app| {
        view(app)["deleted"] != json!("")
    });
    validator_clean(&path);
    click_id(&mut app, "undo-action");
    wait_until(&mut app, "the undo", |app| titles(app).len() == 4);
    assert_eq!(
        spine_names(&path),
        order_before,
        "undo did not return it in place"
    );
    assert_eq!(
        chapter_bytes(&path, doomed_at as u32),
        doomed_bytes,
        "undo changed the chapter"
    );
    validator_clean(&path);

    let file_before_reopen = fs::read(&path).unwrap();
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");
    drop(app);
    let mut again = App::launch_with_data_opt(Some(&path), &data);
    again.wait_first_text();
    install(&mut again);
    wait_until(&mut again, "the contents after a new launch", |app| {
        titles(app).len() == 4
    });
    assert_eq!(fs::read(&path).unwrap(), file_before_reopen);
    assert_eq!(
        fs::read_dir(data.join("books")).unwrap().count(),
        1,
        "the book took a second data folder"
    );
    let status =
        again.js("(async()=>{const ipc=await import('/ipc.js');return ipc.backupStatus()})()");
    assert_eq!(
        status["state"],
        json!("valid"),
        "a later launch lost the backup"
    );
    again.close();
    let _ = fs::remove_dir_all(&dir);
}

fn chapter_bytes(path: &Path, ix: u32) -> Vec<u8> {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    match reader.extract(jadeslip_core::ChapterIx(ix)).unwrap() {
        jadeslip_core::Extracted::Document(bytes) => bytes,
        jadeslip_core::Extracted::Image { bytes, .. } => bytes,
    }
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn search_and_a_replace_preview_follow_the_book_after_an_op() {
    let dir = new_scratch("search");
    let path = dir.join("copy.epub");
    fs::copy(generated_book("toc-nav-only"), &path).unwrap();
    let mut app = App::launch(Some(&path), &[]);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    let added = app.js("(async()=>{const ipc=await import('/ipc.js');const r=await ipc.manageChapters({kind:'add',after:0,title:'Zanzibar Marker'},null);return r.landing})()");
    assert_eq!(added, json!(1));
    let found = app.js("(async()=>{const ipc=await import('/ipc.js');await ipc.searchStart(900,{text:'Zanzibar Marker',regex:false,caseSensitive:false,wholeWord:false},{kind:'whole'});let groups=[];for(;;){const b=await ipc.searchNext(900);groups=groups.concat(b.groups.map((g)=>g.ix));if(b.done)return groups}})()");
    assert_eq!(found, json!([1]), "search did not see the new chapter");
    let stale = app.js("(async()=>{const ipc=await import('/ipc.js');await ipc.replaceStart('pane',901,{kind:'text',text:'the',regex:false,caseSensitive:false,replacement:'THE'},{kind:'whole'});for(;;){const b=await ipc.replaceNext('pane',901);if(b.done)break}await ipc.manageChapters({kind:'add',after:null,title:'Another'},null);await ipc.applyStart('pane',901,902,[]);for(;;){const a=await ipc.applyNext(902);if(a.end)return a.end}})()");
    assert_eq!(
        stale["kind"],
        json!("failed"),
        "a preview survived an op: {stale}"
    );
    assert_eq!(stale["reason"], json!("stale"), "{stale}");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

const SCROLL_FRAMES: &str = r"((frames) => new Promise((done) => {
  const list = document.getElementById('toc-list');
  const deltas = [];
  let last = performance.now();
  let frame = 0;
  const tick = (now) => {
    deltas.push(now - last);
    last = now;
    frame += 1;
    if (frame > frames) { done(deltas.slice(5).sort((a, b) => a - b)); return; }
    list.scrollTop += 170;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}))";

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_rail_stays_smooth_after_a_reorder_on_the_largest_book() {
    let dir = new_scratch("smooth");
    let path = dir.join("copy.epub");
    fs::copy(largest_real_book(), &path).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&path, perms).unwrap();
    let mut app = App::launch(Some(&path), &[]);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    for round in 0..3 {
        let took = app.js("(async()=>{const ipc=await import('/ipc.js');const a=performance.now();await ipc.manageChapters({kind:'move',row:{entry:10},to:{row:{entry:3000},after:true}},null);const b=performance.now();const t=await ipc.loadToc();const c=performance.now();return {op:b-a,toc:c-b,entries:t.entries.length}})()");
        app.js("(async()=>{await (await import('/main.js')).goTo(100)})()");
        let frames = app.js(&format!("{SCROLL_FRAMES}(200)"));
        let values: Vec<f64> = frames
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let p50 = values[values.len() / 2];
        let p99 = values[values.len() * 99 / 100];
        println!("round {round}: {took}, rail scroll frames p50 {p50:.1} p99 {p99:.1} ms");
        assert!(
            p50 < 18.0 && p99 < 30.0,
            "the rail is not smooth: p50 {p50} p99 {p99}"
        );
    }
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn hostile_labels_stay_text_in_the_chapter_controls() {
    let dir = new_scratch("hostile");
    let path = dir.join("copy.epub");
    fs::copy(generated_book("toc-hostile"), &path).unwrap();
    let mut app = App::launch(Some(&path), &[]);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    let hostile = "<img src=x onerror=window.__TAURI_INTERNALS__.invoke('pwned')>";
    app.js(&format!(
        "(async()=>{{const ipc=await import('/ipc.js');await ipc.manageChapters({{kind:'add',after:null,title:{0}}},null);await ipc.manageChapters({{kind:'delete',chapter:0}},null);}})()",
        json!(hostile)
    ));
    app.js("(async()=>{const m=await import('/main.js');await m.goTo(0);return 1})()");
    let controls = app.js("document.querySelectorAll('#toc-list img, #deleted-rows img, #chapter-dialog img, #undo-bar img, #palette img').length");
    assert_eq!(controls, json!(0), "markup was parsed in a control");
    app.js("document.getElementById('deleted-open').click()");
    wait_until(&mut app, "the deleted list", |app| {
        app.js("!document.getElementById('deleted-dialog').hidden") == json!(true)
    });
    let parsed = app.js("document.querySelectorAll('#deleted-rows img').length");
    assert_eq!(parsed, json!(0));
    let state = app.state();
    assert_eq!(state["pwned"], json!([]), "a payload ran");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

fn copy_of(name: &str, dir: &Path) -> PathBuf {
    let path = dir.join("copy.epub");
    fs::copy(generated_book(name), &path).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&path, perms).unwrap();
    path
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_group_moves_as_one_by_drag_and_its_rows_keep_their_nesting() {
    let dir = new_scratch("group");
    let path = copy_of("toc-nested-nav", &dir);
    let mut app = App::launch(Some(&path), &[]);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    let before_rows = rows(&mut app);
    let toc: Value = app.js("(async()=>(await (await import('/ipc.js')).loadToc()).entries)()");
    let entries = toc.as_array().unwrap();
    let group = entries
        .iter()
        .position(|entry| entry["ix"].is_null())
        .expect("the fixture has a parent with no link");
    let group_depth = entries[group]["depth"].as_u64().unwrap();
    let kids = entries[group + 1..]
        .iter()
        .take_while(|entry| entry["depth"].as_u64().unwrap() > group_depth)
        .count();
    let target = entries.len() - 1;
    assert!(
        group + kids < target,
        "the fixture needs a row after the group"
    );
    let names = spine_names(&path);
    let mut moving: Vec<String> = entries[group + 1..=group + kids]
        .iter()
        .filter_map(|entry| entry["ix"].as_u64())
        .map(|ix| names[ix as usize].clone())
        .collect();
    moving.dedup();
    assert!(!moving.is_empty());
    let from = before_rows[group].clone();
    let to = before_rows[target].clone();
    drag_row(&mut app, &from, &to, true);
    wait_until(&mut app, "the group to land", |_| {
        spine_names(&path) != names
    });
    let now = spine_names(&path);
    let at = now.iter().position(|name| *name == moving[0]).unwrap();
    assert_eq!(
        now[at..at + moving.len()],
        moving[..],
        "the group did not stay together"
    );
    assert_eq!(
        now[now.len() - moving.len()..],
        moving[..],
        "the group is not last"
    );
    wait_until(&mut app, "the rail after the move", |app| {
        let toc = app.js("(async()=>(await (await import('/ipc.js')).loadToc()).entries)()");
        let entries = toc.as_array().unwrap().clone();
        let moved_to = entries.iter().rposition(|entry| entry["ix"].is_null());
        moved_to.is_some_and(|at| {
            entries[at + 1..]
                .iter()
                .all(|entry| entry["depth"].as_u64().unwrap() > 0)
        })
    });
    validator_clean(&path);
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

fn rail_shape(app: &mut App) -> Vec<(String, String)> {
    rows(app)
        .iter()
        .map(|row| {
            (
                row["t"].as_str().unwrap().to_owned(),
                row["chev"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn row_titled(app: &mut App, title: &str) -> Value {
    rows(app)
        .into_iter()
        .find(|row| row["t"] == json!(title))
        .unwrap_or_else(|| panic!("no row titled {title}"))
}

fn shape(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(title, chev)| ((*title).to_owned(), (*chev).to_owned()))
        .collect()
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn folded_groups_stay_folded_across_a_move_and_a_rename() {
    let dir = new_scratch("fold");
    let path = copy_of("toc-nested-nav", &dir);
    let mut app = App::launch(Some(&path), &[]);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| titles(app).len() == 10);
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    for group in ["Chapter Two", "Part Two"] {
        let chevron = row_titled(&mut app, group)["fold"].clone();
        app.click(chevron["x"].as_f64().unwrap(), chevron["y"].as_f64().unwrap());
        wait_until(&mut app, "the fold", |app| {
            row_titled(app, group)["chev"] == json!("›")
        });
    }
    let folded = shape(&[
        ("Part One", "⌄"),
        ("Chapter One", ""),
        ("Chapter Two", "›"),
        ("Part Two", "›"),
        ("Chapter Six", ""),
    ]);
    assert_eq!(rail_shape(&mut app), folded);

    let names = spine_names(&path);
    let last = row_titled(&mut app, "Chapter Six");
    app.click(last["x"].as_f64().unwrap(), last["y"].as_f64().unwrap());
    wait_until(&mut app, "the last chapter", |app| {
        view(app)["where"] == json!("Position 6 of 6")
    });
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    app.key_with(KEY_UP, MASK_ALT);
    wait_until(&mut app, "the move", |_| spine_names(&path) != names);
    wait_until(&mut app, "the rail after the move", |app| {
        let now = titles(app);
        let at = |title: &str| now.iter().position(|seen| seen == title);
        at("Chapter Six") < at("Part Two")
    });
    assert_eq!(
        rail_shape(&mut app),
        shape(&[
            ("Part One", "⌄"),
            ("Chapter One", ""),
            ("Chapter Two", "›"),
            ("Chapter Six", ""),
            ("Part Two", "›"),
        ]),
        "a move unfolded a group"
    );

    let second = row_titled(&mut app, "Chapter Two");
    app.click(second["x"].as_f64().unwrap(), second["y"].as_f64().unwrap());
    wait_until(&mut app, "the second chapter", |app| {
        view(app)["where"] == json!("Position 2 of 6")
    });
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    app.key(KEY_F2);
    wait_until(&mut app, "the rename field", |app| {
        view(app)["rename"] == json!(true)
    });
    app.js("document.getElementById('rail-rename').select()");
    app.type_text("Renamed");
    app.key(KEY_ENTER);
    wait_until(&mut app, "the renamed row", |app| {
        titles(app).iter().any(|title| title == "Renamed")
    });
    assert_eq!(
        rail_shape(&mut app),
        shape(&[
            ("Part One", "⌄"),
            ("Chapter One", ""),
            ("Renamed", "›"),
            ("Chapter Six", ""),
            ("Part Two", "›"),
        ]),
        "a rename unfolded a group"
    );
    validator_clean(&path);
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_last_chapter_is_refused_and_a_deleted_chapter_comes_back_from_the_list() {
    let dir = new_scratch("restore");
    let path = copy_of("toc-nav-only", &dir);
    let data = dir.join("data");
    let mut app = App::launch_with_data_opt(Some(&path), &data);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    let original = spine_names(&path);
    let ipc = "const ipc=await import('/ipc.js');";
    app.js(&format!(
        "(async()=>{{{ipc}await ipc.savePlace(3,null)}})()"
    ));
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    app.js("(async()=>(await import('/main.js')).goTo(1))()");
    wait_until(&mut app, "the second chapter", |app| {
        view(app)["where"] == json!("Position 2 of 6")
    });
    wait_until(&mut app, "read after the turn", |app| {
        view(app)["mode"] == json!("read")
    });
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    let row = rows(&mut app)[1].clone();
    app.move_pointer(
        row["x"].as_f64().unwrap() - 60.0,
        row["y"].as_f64().unwrap(),
    );
    wait_until(&mut app, "the row actions", |app| {
        view(app)["actions"] == json!(true)
    });
    click_id(&mut app, "rail-delete");
    wait_until(&mut app, "the question", |app| {
        view(app)["asking"] == json!(true)
    });
    click_id(&mut app, "chapter-dialog-ok");
    wait_until(&mut app, "the row to leave", |app| titles(app).len() == 5);
    wait_until(&mut app, "the deleted list", |app| {
        view(app)["deleted"] != json!("")
    });
    assert_eq!(
        view(&mut app)["where"],
        json!("Position 2 of 5"),
        "the next chapter opens"
    );
    click_id(&mut app, "deleted-open");
    wait_until(&mut app, "the list dialog", |app| {
        app.js("document.querySelectorAll('#deleted-rows .btn').length") == json!(1)
            && app.js("!document.getElementById('deleted-dialog').hidden") == json!(true)
    });
    let restore = app.js("(()=>{const b=document.querySelector('#deleted-rows .btn').getBoundingClientRect();return {x:b.left+b.width/2,y:b.top+b.height/2}})()");
    app.click(
        restore["x"].as_f64().unwrap(),
        restore["y"].as_f64().unwrap(),
    );
    wait_until(&mut app, "the restored row", |app| titles(app).len() == 6);
    assert_eq!(
        spine_names(&path),
        original,
        "the chapter did not come back in place"
    );

    for _ in 0..5 {
        let done = app.js(&format!(
            "(async()=>{{{ipc}return ipc.manageChapters({{kind:'delete',chapter:0}},null).then((r)=>r.book.chapters,(e)=>String(e))}})()"
        ));
        assert!(done.is_number(), "a delete was refused early: {done}");
    }
    let refused = app.js(&format!(
        "(async()=>{{{ipc}return ipc.manageChapters({{kind:'delete',chapter:0}},null).then(()=>'deleted',(e)=>String(e))}})()"
    ));
    assert!(
        refused.as_str().unwrap().contains("last one"),
        "the last chapter was not refused readably: {refused}"
    );
    assert_eq!(spine_names(&path).len(), 1);
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_saved_place_survives_an_op_and_a_new_launch() {
    let dir = new_scratch("place");
    let path = copy_of("toc-nav-only", &dir);
    let data = dir.join("data");
    let mut app = App::launch_with_data_opt(Some(&path), &data);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    app.js("(async()=>(await import('/main.js')).goTo(3))()");
    wait_until(&mut app, "chapter 4", |app| {
        view(app)["where"] == json!("Position 4 of 6")
    });
    app.js("(async()=>{const ipc=await import('/ipc.js');await ipc.savePlace(3,null)})()");
    let shown_name = spine_names(&path)[3].clone();
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    app.key_with(KEY_UP, MASK_ALT);
    wait_until(&mut app, "the shown chapter to move", |app| {
        view(app)["where"] == json!("Position 3 of 6")
    });
    assert_eq!(spine_names(&path)[2], shown_name);
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");
    app.js("(async()=>(await import('/main.js')).goTo(4))()");
    wait_until(&mut app, "chapter 5", |app| {
        view(app)["where"] == json!("Position 5 of 6")
    });
    app.js("(async()=>{const ipc=await import('/ipc.js');await ipc.savePlace(4,null)})()");
    app.close();
    let mut again = App::launch_with_data_opt(Some(&path), &data);
    again.wait_first_text();
    install(&mut again);
    assert_eq!(
        view(&mut again)["where"],
        json!("Position 5 of 6"),
        "a place saved after an op was lost"
    );
    assert_eq!(
        fs::read_dir(data.join("books")).unwrap().count(),
        1,
        "one book keeps one data folder"
    );
    again.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_new_book_from_an_open_book_goes_into_the_library_without_a_picker() {
    let dir = new_scratch("from-open");
    let path = copy_of("toc-nav-only", &dir);
    let before = fs::read(&path).unwrap();
    let mut app = App::launch(Some(&path), &[]);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    app.js("document.getElementById('newbook-dialog').hidden = false");
    set_field(&mut app, "newbook-name", "Second");
    set_field(&mut app, "newbook-first", "Opening");
    click_id(&mut app, "newbook-create");
    wait_mode(&mut app, "edit");
    wait_until(&mut app, "the new rail", |app| {
        titles(app) == vec!["Opening".to_owned()]
    });
    assert_eq!(view(&mut app)["crumb"], json!("Second"));
    assert_eq!(
        app.js("document.getElementById('add-to-shelf').hidden"),
        json!(true),
        "a new book is already on the shelf"
    );
    let made = library_books(app.data_dir());
    assert_eq!(made.len(), 1);
    assert!(made[0].file_name().unwrap().to_string_lossy().starts_with("Second - "));
    validator_clean(&made[0]);
    assert_eq!(fs::read(&path).unwrap(), before, "the open book is not touched");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_search_pane_reruns_after_an_op_so_its_rows_never_point_at_the_wrong_chapter() {
    let dir = new_scratch("pane");
    let path = copy_of("toc-nav-only", &dir);
    let mut app = App::launch(Some(&path), &[]);
    app.wait_first_text();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    let results =
        "[...document.querySelectorAll('#search-results .row, #search-results button')].length";
    app.key_with(0x66, 4);
    wait_until(&mut app, "the pane", |app| {
        app.js("!document.getElementById('search-pane').hidden") == json!(true)
    });
    app.type_text("Zanzibar");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        app.js(results),
        json!(0),
        "a word that is not in the book yet"
    );
    app.key(KEY_ESCAPE);
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    click_id(&mut app, "add-chapter");
    wait_until(&mut app, "the new chapter", |app| {
        view(app)["where"] == json!("Position 2 of 7")
    });
    app.key(KEY_F2);
    wait_until(&mut app, "the rename field", |app| {
        view(app)["rename"] == json!(true)
    });
    app.js("document.getElementById('rail-rename').select()");
    app.type_text("Zanzibar");
    app.key(KEY_ENTER);
    wait_until(&mut app, "the renamed chapter", |app| {
        titles(app)[1] == "Zanzibar"
    });
    app.key_with(0x66, 4);
    wait_until(&mut app, "search to find the new chapter", |app| {
        app.js(
            "(async()=>{const m=document.getElementById('search-count').textContent;return m})()",
        )
        .as_str()
        .unwrap_or_default()
        .contains('1')
    });
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

fn windows_of(pid: u32) -> Vec<Value> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .expect("hyprctl runs");
    let clients: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    clients
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|client| client["pid"].as_u64() == Some(u64::from(pid)))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn monitor_id_of_output(name: &str) -> i64 {
    let output = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .expect("hyprctl runs");
    let monitors: Value = serde_json::from_slice(&output.stdout).unwrap();
    monitors
        .as_array()
        .unwrap()
        .iter()
        .find(|monitor| monitor["name"] == json!(name))
        .and_then(|monitor| monitor["id"].as_i64())
        .expect("the test output exists")
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_native_save_picker_opens_on_the_test_output() {
    let dir = new_scratch("picker");
    let book = copy_of("toc-nav-only", &dir);
    let mut app = App::launch(Some(&book), &[]);
    app.wait_first_text();
    app.assert_on_headless_output();
    install(&mut app);
    wait_until(&mut app, "the contents", |app| !titles(app).is_empty());
    app.op(&json!({"op": "arm_save_answer", "real": true}));
    click_id(&mut app, "open-palette");
    wait_until(&mut app, "the palette", |app| {
        app.js("!document.getElementById('palette').hidden") == json!(true)
    });
    app.js("(()=>{const input=document.getElementById('palette-input');input.value='Export a copy';input.dispatchEvent(new Event('input'))})()");
    app.key(KEY_ENTER);
    let test_output = monitor_id_of_output(&jadeslip_e2e::headless_output());
    let deadline = Instant::now() + Duration::from_secs(15);
    let windows = loop {
        let windows = windows_of(app.pid);
        for window in &windows {
            assert_eq!(
                window["monitor"].as_i64(),
                Some(test_output),
                "a window opened off the test output"
            );
        }
        if windows.len() >= 2 {
            break windows;
        }
        assert!(Instant::now() < deadline, "the save picker never opened");
        std::thread::sleep(Duration::from_millis(5));
    };
    println!("{} windows on the test output", windows.len());
    assert!(
        windows
            .iter()
            .any(|window| window["title"] == json!("Export a copy")),
        "the picker has its title"
    );
    drop(app);
    let _ = fs::remove_dir_all(&dir);
}
